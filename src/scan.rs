//! Scan orchestration — spec §4.1 (confidence), §6 (detection), §6.5 (ranking), §14 (determinism).

use std::collections::BTreeSet;
use std::path::Path;

use crate::diag;
use crate::entrypoint::{self, Resolution};
use crate::fileset::{self, FileSet};
use crate::manifest::{self, PyProject};
use crate::model::*;
use crate::workspace;

pub struct ScanOptions {
    pub target_dir: Option<String>,
    pub entrypoint: Option<String>,
    pub max_files: Option<u64>,
    pub follow_symlinks: bool,
    pub extra_ignore_files: Vec<String>,
}

pub fn scan(root: &Path, opts: &ScanOptions) -> ScanResult {
    let effective = workspace::discover_upward(root);
    let fs = fileset::walk_fs(
        &effective.walk_root,
        opts.max_files,
        opts.follow_symlinks,
        &opts.extra_ignore_files,
    );
    let mut scan_diags: Vec<Diagnostic> = Vec::new();
    if fs.truncated {
        scan_diags.push(diag::kb802(opts.max_files.unwrap_or(0)));
    }

    // ── workspace at the effective root ────────────────────────────────
    let root_pyproject = fs.read_str("pyproject.toml");
    let root_parsed: Option<PyProject> = root_pyproject
        .as_deref()
        .and_then(|s| manifest::parse_pyproject(s).ok());
    let ws_table = root_parsed
        .as_ref()
        .and_then(|pp| pp.tool.as_ref())
        .and_then(|t| t.uv.as_ref())
        .and_then(|u| u.workspace.clone());
    let mut ws_info = None;
    if let Some(ws) = &ws_table {
        let has_project = root_parsed.as_ref().is_some_and(|pp| pp.project.is_some());
        let info = workspace::expand_workspace(&fs, ws, has_project);
        scan_diags.extend(info.diagnostics.clone());
        ws_info = Some(info.workspace);
    }

    // ── project discovery (§6.2) ───────────────────────────────────────
    let pyproject_dirs = fs.dirs_with("pyproject.toml");
    let mut project_dirs: BTreeSet<String> = pyproject_dirs.iter().cloned().collect();
    // requirements/Pipfile/setup.py projects: not nested under a pyproject project
    for marker in ["requirements.txt", "Pipfile", "setup.py"] {
        for dir in fs.dirs_with(marker) {
            let nested = pyproject_dirs
                .iter()
                .any(|p| p == &dir || (dir.starts_with(p.as_str()) && !p.is_empty()));
            if !nested {
                project_dirs.insert(dir);
            }
        }
    }
    // bare-scripts root project: *.py at root, no manifests anywhere
    if project_dirs.is_empty() && fs.files.keys().any(|f| f.ends_with(".py")) {
        project_dirs.insert(String::new());
    }

    // ── hint validation: target_dir (§10) ──────────────────────────────
    let mut hint_dir: Option<String> = None;
    if let Some(raw) = &opts.target_dir {
        let normalized = normalize_rel(raw);
        if normalized.starts_with("..") {
            scan_diags.push(diag::kb501(raw));
        } else {
            let as_project = if normalized == "." {
                String::new()
            } else {
                normalized.clone()
            };
            let exists = fs
                .files
                .keys()
                .any(|f| as_project.is_empty() || f.starts_with(&format!("{as_project}/")));
            if !exists {
                scan_diags.push(diag::kb500(raw));
            } else if !project_dirs.contains(&as_project) {
                scan_diags.push(diag::kb502(raw));
            } else {
                hint_dir = Some(as_project);
            }
        }
    }

    // ── per-project analysis ───────────────────────────────────────────
    let workspace_lock = fs.contains("uv.lock");
    let mut projects: Vec<Project> = Vec::new();
    for dir in &project_dirs {
        let hint_entry = if hint_dir.as_deref() == Some(dir.as_str())
            || (hint_dir.is_none() && origin_matches(&effective.scan_origin, dir))
        {
            opts.entrypoint.as_deref()
        } else {
            None
        };
        projects.push(analyze_project(&fs, dir, workspace_lock, hint_entry));
    }

    // ── flatten + rank targets (§6.5) ──────────────────────────────────
    let mut targets: Vec<DeployTarget> = projects
        .iter()
        .flat_map(|p| p.deploy_targets.iter().cloned())
        .collect();
    let origin = hint_dir.clone().unwrap_or_else(|| {
        if effective.scan_origin == "." {
            String::new()
        } else {
            effective.scan_origin.clone()
        }
    });
    let roles_map: std::collections::BTreeMap<String, Vec<String>> = projects
        .iter()
        .map(|p| (p.path.clone(), p.roles.clone()))
        .collect();
    let roles_of = |pp: &str| roles_map.get(pp).cloned().unwrap_or_default();
    targets.sort_by_key(|t| {
        let affinity = origin_matches(
            &(if origin.is_empty() {
                ".".to_string()
            } else {
                origin.clone()
            }),
            &(if t.project_path == "." {
                String::new()
            } else {
                t.project_path.clone()
            }),
        );
        let example = roles_of(&t.project_path)
            .iter()
            .any(|r| r == "example" || r == "test-support");
        (
            !affinity,                           // affinity first (§6.5.1)
            example,                             // non-example first (§6.5.2)
            t.form != "project",                 // project form first (§6.5.3)
            confidence_rank(&t.confidence),      // high first (§6.5.4)
            t.project_path.matches('/').count(), // shallower first (§6.5.5)
            t.project_path.clone(),              // byte order (§6.5.6-7)
        )
    });
    if let Some(first) = targets.first_mut() {
        first.recommended = true;
    }
    // mirror recommended back into the nested copies
    for project in &mut projects {
        for target in &mut project.deploy_targets {
            target.recommended = targets.first().is_some_and(|t| {
                t.project_path == target.project_path && t.framework == target.framework
            });
        }
    }

    // ── top-level ambiguity / origin diagnostics (§6.3, §6.5) ──────────
    let non_example: Vec<&DeployTarget> = targets
        .iter()
        .filter(|t| {
            !roles_of(&t.project_path)
                .iter()
                .any(|r| r == "example" || r == "test-support")
        })
        .collect();
    let origin_affine = targets.iter().any(|t| {
        origin_matches(
            &(if origin.is_empty() {
                ".".to_string()
            } else {
                origin.clone()
            }),
            &(if t.project_path == "." {
                String::new()
            } else {
                t.project_path.clone()
            }),
        )
    });
    if non_example.len() >= 2
        && confidence_rank(&non_example[0].confidence)
            == confidence_rank(&non_example[1].confidence)
        && !origin_affine
    {
        let paths: Vec<&str> = non_example
            .iter()
            .take(4)
            .map(|t| t.project_path.as_str())
            .collect();
        scan_diags.push(diag::kb110(&paths));
    }
    if !origin.is_empty() && !origin_affine && !targets.is_empty() {
        scan_diags.push(diag::kb115(&origin));
    }

    // ── outcome codes (§6.4) ───────────────────────────────────────────
    let python_seen = !projects.is_empty() || fs.files.keys().any(|f| f.ends_with(".py"));
    if projects.is_empty() {
        scan_diags.push(diag::kb100());
    } else if targets.is_empty() {
        scan_diags.push(diag::kb102());
    }

    // ── classification (§11) ───────────────────────────────────────────
    let declared_fastapi = projects
        .iter()
        .any(|p| p.frameworks.iter().any(|f| f == "fastapi"));
    // weak signals: convention-only targets, or setup.py string-scan mentions
    let likely = targets.iter().any(|t| t.framework == "fastapi")
        || projects.iter().any(|p| {
            p.evidence
                .iter()
                .any(|e| e.detail.contains("setup.py mentions fastapi"))
        });
    let classification = Classification {
        python: if python_seen { "yes" } else { "no" }.to_string(),
        uses_fastapi: if declared_fastapi {
            "yes"
        } else if likely {
            "likely"
        } else {
            "no"
        }
        .to_string(),
        primary: targets.first().map(|t| ClassificationPrimary {
            path: t.project_path.clone(),
            evidence: t
                .evidence
                .first()
                .map(|e| format!("{}: {}", e.kind, e.detail))
                .unwrap_or_default(),
        }),
    };

    // ── aggregate + order deterministically (§14) ──────────────────────
    let mut all_diags = scan_diags;
    for project in &projects {
        all_diags.extend(project.diagnostics.iter().cloned());
        for target in &project.deploy_targets {
            all_diags.extend(target.diagnostics.iter().cloned());
        }
    }
    dedup_sort_diags(&mut all_diags);

    let files_seen = fs.files.len() as u64;
    projects.sort_by(|a, b| a.path.cmp(&b.path));

    ScanResult {
        schema_version: SCHEMA_VERSION,
        root: root.to_string_lossy().to_string(),
        upload_root: effective.upload_root,
        scan_origin: effective.scan_origin,
        status: "complete".to_string(),
        want_files: Vec::new(),
        input: InputInfo {
            mode: "fs".to_string(),
            files_seen,
            complete: !fs.truncated,
        },
        workspace: ws_info,
        projects,
        deploy_targets: targets,
        classification,
        diagnostics: all_diags,
    }
}

// ── per-project ────────────────────────────────────────────────────────────

fn analyze_project(
    fs: &FileSet,
    dir: &str,
    workspace_lock: bool,
    entrypoint_hint: Option<&str>,
) -> Project {
    let display_path = if dir.is_empty() {
        ".".to_string()
    } else {
        dir.to_string()
    };
    let mut diagnostics: Vec<Diagnostic> = Vec::new();
    let mut evidence: Vec<Evidence> = Vec::new();

    let files = manifest::project_files(fs, dir);
    let mut parsed: Option<PyProject> = None;
    if let Some(pp_path) = &files.pyproject {
        match fs.read_str(pp_path).map(|s| manifest::parse_pyproject(&s)) {
            Some(Ok(pp)) => parsed = Some(pp),
            Some(Err(err)) => diagnostics.push(diag::kb201(pp_path, &err)),
            None => {}
        }
    }
    let is_ws_root = parsed
        .as_ref()
        .and_then(|pp| pp.tool.as_ref())
        .and_then(|t| t.uv.as_ref())
        .is_some_and(|u| u.workspace.is_some());
    if let (Some(pp_path), Some(pp)) = (&files.pyproject, &parsed) {
        if pp.project.is_none() && !is_ws_root {
            diagnostics.push(diag::kb202(pp_path));
        }
    }

    // declared dependencies (§8)
    let mut declared: Vec<DeclaredDep> = Vec::new();
    if let (Some(pp), Some(pp_path)) = (&parsed, &files.pyproject) {
        declared.extend(manifest::pyproject_deps(pp, pp_path));
    }
    for req in &files.requirements {
        manifest::requirements_deps(fs, req, 0, &mut declared);
    }
    let pipfile_path = if dir.is_empty() {
        "Pipfile".to_string()
    } else {
        format!("{dir}/Pipfile")
    };
    if fs.contains(&pipfile_path) {
        if let Some(source) = fs.read_str(&pipfile_path) {
            declared.extend(manifest::pipfile_deps(&source, &pipfile_path));
        }
    }
    declared.sort_by(|a, b| (&a.name, &a.source.path).cmp(&(&b.name, &b.source.path)));

    // resolved from lockfiles (§8)
    let mut resolved: Vec<ResolvedDep> = Vec::new();
    for lock in &files.lockfiles {
        if lock.parsed {
            if let Some(source) = fs.read_str(&lock.path) {
                resolved.extend(manifest::parse_lock_resolved(
                    &source, &lock.path, &lock.kind,
                ));
            }
        }
    }

    // KB300 / KB305 / package manager + KB306
    let has_project_deps = parsed
        .as_ref()
        .and_then(|pp| pp.project.as_ref())
        .and_then(|p| p.dependencies.as_ref())
        .is_some_and(|d| !d.is_empty());
    let has_root_requirements = files
        .requirements
        .iter()
        .any(|r| r.rsplit('/').next() == Some("requirements.txt"));
    if has_project_deps && has_root_requirements {
        diagnostics.push(diag::kb300(&display_path));
    }
    if files.lockfiles.len() > 1 {
        let names: Vec<String> = files.lockfiles.iter().map(|l| l.path.clone()).collect();
        diagnostics.push(diag::kb305(&display_path, &names));
    }
    let has_poetry_table = parsed
        .as_ref()
        .and_then(|pp| pp.tool.as_ref())
        .is_some_and(|t| t.poetry.is_some());
    let has_pdm_table = parsed
        .as_ref()
        .and_then(|pp| pp.tool.as_ref())
        .is_some_and(|t| t.pdm.is_some());
    let build_backend = parsed
        .as_ref()
        .and_then(|pp| pp.build_system.as_ref())
        .and_then(|b| b.build_backend.clone())
        .unwrap_or_default();
    let package_manager =
        detect_package_manager(&files, has_poetry_table, has_pdm_table, &build_backend);
    if matches!(package_manager, "poetry" | "pipenv" | "pdm") {
        diagnostics.push(diag::kb306(&display_path, package_manager));
    }

    // frameworks (Layer 1, §6.2) with group provenance
    let mut frameworks: Vec<String> = Vec::new();
    let mut fastapi_groups: Vec<String> = Vec::new();
    for dep in &declared {
        if let Some(fw) = manifest::framework_for(&dep.name) {
            if !frameworks.contains(&fw.to_string()) {
                frameworks.push(fw.to_string());
            }
            evidence.push(Evidence {
                kind: "dependency-declared".to_string(),
                path: dep.source.path.clone(),
                span: None,
                detail: format!("{} in `{}`", dep.raw, dep.group),
            });
            if fw == "fastapi" {
                fastapi_groups.push(dep.group.clone());
            }
        }
    }
    // django marker file (§12 identity-only)
    let manage = if dir.is_empty() {
        "manage.py".to_string()
    } else {
        format!("{dir}/manage.py")
    };
    if fs.contains(&manage) {
        if let Some(src) = fs.read_str(&manage) {
            if src.contains("DJANGO_SETTINGS_MODULE") && !frameworks.contains(&"django".to_string())
            {
                frameworks.push("django".to_string());
                evidence.push(Evidence {
                    kind: "marker-file".to_string(),
                    path: manage,
                    span: None,
                    detail: "manage.py sets DJANGO_SETTINGS_MODULE".to_string(),
                });
            }
        }
    }
    let setup_py = if dir.is_empty() {
        "setup.py".to_string()
    } else {
        format!("{dir}/setup.py")
    };
    if fs.contains(&setup_py) {
        if let Some(source) = fs.read_str(&setup_py) {
            for name in manifest::setup_py_framework_mentions(&source) {
                evidence.push(Evidence {
                    kind: "marker-file".to_string(),
                    path: setup_py.clone(),
                    span: None,
                    detail: format!("setup.py mentions {name} (string scan; not executed)"),
                });
            }
        }
    }
    frameworks.sort();
    if frameworks.len() > 1 {
        diagnostics.push(diag::kb101(&display_path, &frameworks));
    }

    // evident-install-path rule (§6.2): which fastapi declarations install?
    let has_lock = workspace_lock || files.lockfiles.iter().any(|l| l.kind == "uv");
    let installable = |group: &str| -> bool {
        if has_lock {
            group == "project" || group == "dev" || group == "group:dev"
        } else {
            // pyproject path installs only [project.dependencies]; the
            // requirements path records its non-dev files as "project"
            group == "project"
        }
    };
    let fastapi_declared = !fastapi_groups.is_empty();
    let fastapi_installable = fastapi_groups.iter().any(|g| installable(g));
    let mut dep_cap = false;
    if fastapi_declared && !fastapi_installable {
        dep_cap = true;
        for group in &fastapi_groups {
            diagnostics.push(diag::kb301(&display_path, "fastapi", group));
        }
        diagnostics.push(diag::kb307(&display_path, "fastapi"));
    }

    // python version facts (§8) + KB700
    let requires_python = parsed
        .as_ref()
        .and_then(|pp| pp.project.as_ref())
        .and_then(|p| p.requires_python.clone());
    let mut version_pins = Vec::new();
    let pin_path = if dir.is_empty() {
        ".python-version".to_string()
    } else {
        format!("{dir}/.python-version")
    };
    if let Some(pin) = fs.read_str(&pin_path) {
        let pin = pin.trim().to_string();
        if !pin.is_empty() {
            if let (Some(req), Ok(version)) = (
                requires_python
                    .as_deref()
                    .and_then(|r| r.parse::<pep440_rs::VersionSpecifiers>().ok()),
                pin.parse::<pep440_rs::Version>(),
            ) {
                if !req.contains(&version) {
                    diagnostics.push(diag::kb700(
                        &display_path,
                        &format!(
                            ".python-version pins {pin} but requires-python is {}",
                            requires_python.as_deref().unwrap_or("")
                        ),
                    ));
                }
            }
            version_pins.push(VersionPin {
                source: ".python-version".to_string(),
                value: pin,
            });
        }
    }

    // ── Layer 2 (§6.3) ──────────────────────────────────────────────────
    let tool_entrypoint = parsed
        .as_ref()
        .and_then(|pp| pp.tool.as_ref())
        .and_then(|t| t.fastapi.as_ref())
        .and_then(|f| f.entrypoint.clone());

    let mut resolution: Option<Resolution> = None;
    if let Some(hint) = entrypoint_hint {
        match entrypoint::validate_entrypoint(fs, dir, hint, "hint") {
            Ok(res) => resolution = Some(res),
            Err(diags) => diagnostics.extend(diags),
        }
    }
    if resolution.is_none() {
        if let Some(spec) = &tool_entrypoint {
            match entrypoint::validate_entrypoint(fs, dir, spec, "tool-fastapi") {
                Ok(res) => resolution = Some(res),
                Err(diags) => diagnostics.extend(diags),
            }
        }
    }
    let mut router_only = false;
    let mut import_seen = false;
    if resolution.is_none() {
        let scan = entrypoint::resolve_project(fs, dir);
        diagnostics.extend(scan.diagnostics);
        evidence.extend(scan.evidence);
        router_only = scan.router_only;
        import_seen = scan.fastapi_import_seen;
        resolution = scan.resolution;
    }

    // ── target construction + confidence (§4.1) ────────────────────────
    let mut deploy_targets = Vec::new();
    if let Some(res) = resolution {
        let mut caps = 0;
        if res.rule == 4 {
            caps += 1;
        }
        if res.is_factory {
            caps += 1;
        }
        if dep_cap {
            caps += 1;
        }
        let confidence = if !fastapi_declared {
            "low" // convention-only (§4.1)
        } else if caps == 0 {
            "high"
        } else if caps == 1 {
            "medium"
        } else {
            "low"
        };
        let source = match res.rule {
            1 => "hint",
            2 => "tool-fastapi",
            _ => "inferred",
        };
        deploy_targets.push(DeployTarget {
            framework: "fastapi".to_string(),
            form: "project".to_string(),
            project_path: display_path.clone(),
            entrypoint: Some(Entrypoint {
                kind: "asgi".to_string(),
                module: res.module.clone(),
                attribute: res.attribute.clone(),
                is_factory: res.is_factory,
                import_root: res.import_root.clone(),
                source: source.to_string(),
                as_string: format!("{}:{}", res.module, res.attribute),
            }),
            confidence: confidence.to_string(),
            recommended: false,
            env_vars: Vec::new(),
            evidence: res.evidence,
            diagnostics: res.diagnostics,
        });
    } else if fastapi_declared {
        // KB103: framework declared, no app object — placeholder target (§6.4)
        let kb103 = diag::kb103(&display_path, "fastapi");
        deploy_targets.push(DeployTarget {
            framework: "fastapi".to_string(),
            form: "project".to_string(),
            project_path: display_path.clone(),
            entrypoint: None,
            confidence: if dep_cap { "low" } else { "medium" }.to_string(),
            recommended: false,
            env_vars: Vec::new(),
            evidence: Vec::new(),
            diagnostics: vec![kb103],
        });
        if router_only {
            diagnostics.push(diag::kb104(&display_path));
        }
    } else if router_only {
        diagnostics.push(diag::kb104(&display_path));
    }
    let _ = import_seen;

    // roles (§6.1)
    let mut roles = fileset::path_roles(dir);
    let is_webapp = !deploy_targets.is_empty();
    if is_webapp {
        roles.insert(0, "webapp".to_string());
    } else if files.pyproject.is_some() || !files.requirements.is_empty() {
        roles.insert(0, "library".to_string());
    }

    let name = parsed
        .as_ref()
        .and_then(|pp| pp.project.as_ref())
        .and_then(|p| p.name.clone())
        .or_else(|| {
            parsed
                .as_ref()
                .and_then(|pp| pp.tool.as_ref())
                .and_then(|t| t.poetry.as_ref())
                .and_then(|p| p.get("name"))
                .and_then(|n| n.as_str())
                .map(str::to_string)
        });

    evidence.sort_by(|a, b| (&a.path, &a.kind).cmp(&(&b.path, &b.kind)));
    dedup_sort_diags(&mut diagnostics);

    Project {
        path: display_path,
        name,
        roles,
        frameworks,
        deploy_targets,
        dependencies: Some(Dependencies {
            package_manager: package_manager.to_string(),
            manifests: files.manifests,
            lockfiles: files.lockfiles,
            declared,
            resolved,
        }),
        env_vars: Vec::new(),
        python: PythonInfo {
            requires_python,
            version_pins,
        },
        evidence,
        diagnostics,
    }
}

fn detect_package_manager(
    files: &manifest::ProjectFiles,
    has_poetry_table: bool,
    has_pdm_table: bool,
    build_backend: &str,
) -> &'static str {
    let has_kind = |k: &str| files.lockfiles.iter().any(|l| l.kind == k);
    if has_kind("uv") {
        "uv"
    } else if has_kind("poetry") || has_poetry_table || build_backend.starts_with("poetry") {
        "poetry"
    } else if has_kind("pdm") || has_pdm_table || build_backend.starts_with("pdm") {
        "pdm"
    } else if has_kind("pipenv") || files.manifests.iter().any(|m| m.kind == "pipfile") {
        "pipenv"
    } else if files.pyproject.is_some() {
        "uv"
    } else if !files.requirements.is_empty() {
        "pip"
    } else {
        "unknown"
    }
}

fn confidence_rank(confidence: &str) -> u8 {
    match confidence {
        "high" => 0,
        "medium" => 1,
        _ => 2,
    }
}

/// origin is inside (or equal to) the project dir — §6.5 affinity.
/// `origin` uses "." for root; `project_dir` is "" for root.
fn origin_matches(origin: &str, project_dir: &str) -> bool {
    let origin = if origin == "." { "" } else { origin };
    let project = if project_dir == "." { "" } else { project_dir };
    if project.is_empty() {
        return origin.is_empty();
    }
    origin == project || origin.starts_with(&format!("{project}/"))
}

fn normalize_rel(path: &str) -> String {
    let normalized = path.replace('\\', "/");
    let mut parts: Vec<&str> = Vec::new();
    let mut escapes = 0usize;
    for part in normalized.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                if parts.pop().is_none() {
                    escapes += 1;
                }
            }
            p => parts.push(p),
        }
    }
    if escapes > 0 {
        return "..".to_string();
    }
    if parts.is_empty() {
        ".".to_string()
    } else {
        parts.join("/")
    }
}

/// §14: dedup by (code, path, span-start), order by (path, span-start, code).
fn dedup_sort_diags(diags: &mut Vec<Diagnostic>) {
    diags.sort_by(|a, b| {
        let key = |d: &Diagnostic| {
            (
                d.path.clone().unwrap_or_default(),
                d.span
                    .as_ref()
                    .map(|s| (s.start_line, s.start_col))
                    .unwrap_or((0, 0)),
                d.code.clone(),
            )
        };
        key(a).cmp(&key(b))
    });
    diags.dedup_by(|a, b| {
        a.code == b.code
            && a.path == b.path
            && a.span.as_ref().map(|s| (s.start_line, s.start_col))
                == b.span.as_ref().map(|s| (s.start_line, s.start_col))
    });
}
