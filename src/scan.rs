//! Deterministic filesystem scan orchestration and application assembly.

use std::collections::BTreeSet;
use std::path::Path;

use crate::diag;
use crate::entrypoint::{self, Resolution};
use crate::fileset::{self, FileSet};
use crate::manifest::{self, PyProject};
use crate::model::*;
use crate::node::{self, RawNodeDiscovery, RawNodePackage, RawTechnologySignal};
use crate::runtime;
use crate::workspace;

pub struct ScanOptions {
    pub application_dir: Option<String>,
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
    let mut result = scan_fileset(
        &fs,
        opts,
        root.to_string_lossy().to_string(),
        effective.upload_root,
        effective.scan_origin,
    );
    if fs.truncated || fs.unavailable_seen() {
        result.completeness = "partial".to_string();
    }
    result
}

pub fn analyze(fs: &FileSet, inventory_complete: bool) -> ScanResult {
    let opts = ScanOptions {
        application_dir: None,
        entrypoint: None,
        max_files: None,
        follow_symlinks: false,
        extra_ignore_files: Vec::new(),
    };
    if fs.has_ignore_wants() {
        return finish_virtual_result(
            ScanResult {
                schema_version: SCHEMA_VERSION,
                root: ".".to_string(),
                upload_root: ".".to_string(),
                scan_origin: ".".to_string(),
                status: "complete".to_string(),
                completeness: "complete".to_string(),
                want_files: Vec::new(),
                workspace: None,
                applications: Vec::new(),
                diagnostics: Vec::new(),
            },
            fs,
            inventory_complete,
        );
    }
    let result = scan_fileset(fs, &opts, ".".to_string(), ".".to_string(), ".".to_string());
    finish_virtual_result(result, fs, inventory_complete)
}

fn finish_virtual_result(
    mut result: ScanResult,
    fs: &FileSet,
    inventory_complete: bool,
) -> ScanResult {
    result.want_files = fs.wants();
    result.status = if result.want_files.is_empty() {
        "complete".to_string()
    } else {
        "needs_files".to_string()
    };
    let identity_parse_failed = result
        .diagnostics
        .iter()
        .any(|diagnostic| matches!(diagnostic.code.as_str(), "KB201" | "KB203"));
    if !inventory_complete
        || fs.unavailable_seen()
        || identity_parse_failed
        || !result.want_files.is_empty()
    {
        result.completeness = "partial".to_string();
    }
    result
}

fn scan_fileset(
    fs: &FileSet,
    opts: &ScanOptions,
    root: String,
    upload_root: String,
    scan_origin: String,
) -> ScanResult {
    let mut scan_diags: Vec<Diagnostic> = Vec::new();
    for issue in &fs.issues {
        let diagnostic = if issue.message.contains("scan root") {
            diag::kb800(&issue.path, &issue.message)
        } else {
            diag::kb801(&issue.path, &issue.message)
        };
        scan_diags.push(diagnostic);
    }
    if fs.truncated {
        scan_diags.push(diag::kb802(opts.max_files.unwrap_or(0)));
    }
    let node_discovery = node::discover(fs);
    scan_diags.extend(
        node_discovery
            .parse_errors
            .iter()
            .map(|error| diag::kb203(&error.path, &error.message)),
    );
    scan_diags.extend(
        node_discovery
            .packages
            .iter()
            .filter(|package| package.package_manager_candidates.len() > 1)
            .map(|package| diag::kb308(&package.path, &package.package_manager_candidates)),
    );

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
        let info = workspace::expand_workspace(fs, ws, has_project);
        scan_diags.extend(info.diagnostics.clone());
        ws_info = Some(info.workspace);
    }
    if let Some(node_workspace) = node_discovery
        .workspaces
        .iter()
        .find(|workspace| workspace.path == ".")
    {
        for pattern in &node_workspace.unmatched_patterns {
            scan_diags.push(diag::kb402(".", pattern));
        }
        if node_workspace.package_manager_candidates.len() > 1 {
            scan_diags.push(diag::kb308(".", &node_workspace.package_manager_candidates));
        }
        let mut node_members = node_workspace.members.clone();
        if let Some(workspace) = &mut ws_info {
            workspace.kind = "mixed".to_string();
            workspace.members.append(&mut node_members);
            workspace
                .members
                .sort_by(|a, b| (a != ".").cmp(&(b != ".")).then(a.cmp(b)));
            workspace.members.dedup();
        } else {
            let mut members = vec![".".to_string()];
            members.append(&mut node_members);
            members.sort_by(|a, b| (a != ".").cmp(&(b != ".")).then(a.cmp(b)));
            members.dedup();
            ws_info = Some(Workspace {
                kind: node_workspace
                    .package_manager
                    .as_ref()
                    .map(|manager| manager.name.clone())
                    .unwrap_or_else(|| "node".to_string()),
                path: ".".to_string(),
                virtual_root: true,
                members,
            });
        }
    }

    // ── Python project discovery ───────────────────────────────────────
    let pyproject_dirs = fs.dirs_with("pyproject.toml");
    let mut project_dirs: BTreeSet<String> = pyproject_dirs.iter().cloned().collect();
    // requirements/Pipfile/setup.py projects: not nested under a pyproject project
    for marker in ["requirements.txt", "Pipfile", "setup.py"] {
        for dir in fs.dirs_with(marker) {
            let nested = pyproject_dirs
                .iter()
                .any(|p| p == &dir || (!p.is_empty() && dir.starts_with(&format!("{p}/"))));
            if !nested {
                project_dirs.insert(dir);
            }
        }
    }
    // bare-scripts root project: *.py at root, no manifests anywhere
    if project_dirs.is_empty() && fs.files.keys().any(|f| f.ends_with(".py")) {
        project_dirs.insert(String::new());
    }

    // ── hint validation: application_dir ──────────────────────────────
    let mut hint_dir: Option<String> = None;
    if let Some(raw) = &opts.application_dir {
        let normalized = normalize_rel(raw);
        if is_absolute_like(raw) || normalized.starts_with("..") {
            scan_diags.push(diag::kb501(raw));
        } else {
            let scan_origin = if scan_origin == "." {
                String::new()
            } else {
                scan_origin.clone()
            };
            let as_project = if normalized == "." {
                scan_origin
            } else if scan_origin.is_empty() {
                normalized.clone()
            } else {
                format!("{scan_origin}/{normalized}")
            };
            let exists = fs
                .files
                .keys()
                .any(|f| as_project.is_empty() || f.starts_with(&format!("{as_project}/")));
            if !exists {
                scan_diags.push(diag::kb500(raw));
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
            || (hint_dir.is_none() && origin_matches(&scan_origin, dir))
        {
            opts.entrypoint.as_deref()
        } else {
            None
        };
        projects.push(analyze_project(fs, dir, workspace_lock, hint_entry));
    }

    let mut applications = python_applications(&projects);
    merge_node_applications(&mut applications, &node_discovery);
    applications.retain(|application| {
        application
            .technologies
            .iter()
            .any(|technology| technology.role == "primary")
    });
    if let Some(workspace) = &mut ws_info {
        workspace.virtual_root = !applications
            .iter()
            .any(|application| application.application_dir == ".");
    }
    if let Some(hint) = &hint_dir {
        let display = if hint.is_empty() { "." } else { hint };
        if !applications
            .iter()
            .any(|application| application.application_dir == display)
        {
            scan_diags.push(diag::kb502(
                opts.application_dir.as_deref().unwrap_or(display),
            ));
        }
    }

    // ── aggregate + order deterministically ────────────────────────────
    let mut all_diags = scan_diags;
    for project in &projects {
        all_diags.extend(project.diagnostics.iter().cloned());
        for target in &project.deploy_targets {
            all_diags.extend(target.diagnostics.iter().cloned());
        }
    }
    for application in &applications {
        all_diags.extend(application.diagnostics.iter().cloned());
    }
    dedup_sort_diags(&mut all_diags);

    applications.sort_by(|a, b| a.application_dir.cmp(&b.application_dir));

    if applications.is_empty() {
        all_diags.push(
            if projects.is_empty() && node_discovery.packages.is_empty() {
                diag::kb100()
            } else {
                diag::kb102()
            },
        );
        dedup_sort_diags(&mut all_diags);
    }

    ScanResult {
        schema_version: SCHEMA_VERSION,
        root,
        upload_root,
        scan_origin,
        status: "complete".to_string(),
        completeness: "complete".to_string(),
        want_files: Vec::new(),
        workspace: ws_info,
        applications,
        diagnostics: all_diags,
    }
}

fn python_applications(projects: &[Project]) -> Vec<Application> {
    projects
        .iter()
        .filter(|project| project.is_python_project)
        .map(|project| {
            let mut technologies = vec![Technology {
                name: "python".to_string(),
                kind: "language".to_string(),
                role: "supporting".to_string(),
                confidence: "high".to_string(),
                evidence: project.evidence.clone(),
            }];
            let mut frameworks = project.frameworks.clone();
            frameworks.extend(
                project
                    .deploy_targets
                    .iter()
                    .map(|target| target.framework.clone()),
            );
            frameworks.sort();
            frameworks.dedup();
            for framework in &frameworks {
                let target = project
                    .deploy_targets
                    .iter()
                    .find(|target| &target.framework == framework);
                technologies.push(Technology {
                    name: framework.clone(),
                    kind: "framework".to_string(),
                    role: "primary".to_string(),
                    confidence: target
                        .map(|target| target.confidence.clone())
                        .unwrap_or_else(|| "high".to_string()),
                    evidence: target
                        .map(|target| target.evidence.clone())
                        .unwrap_or_else(|| project.evidence.clone()),
                });
            }
            technologies
                .sort_by(|a, b| (&a.role, &a.kind, &a.name).cmp(&(&b.role, &b.kind, &b.name)));

            let entrypoint = project
                .deploy_targets
                .iter()
                .find_map(|target| target.entrypoint.clone());
            let mut diagnostics = project.diagnostics.clone();
            for target in &project.deploy_targets {
                diagnostics.extend(target.diagnostics.clone());
            }
            dedup_sort_diags(&mut diagnostics);

            Application {
                application_dir: project.path.clone(),
                name: project.name.clone(),
                technologies,
                entrypoint,
                dependencies: project.dependencies.clone().into_iter().collect(),
                build_scripts: Vec::new(),
                env_vars: project.env_vars.clone(),
                python: Some(project.python.clone()),
                node: None,
                evidence: project.evidence.clone(),
                diagnostics,
            }
        })
        .collect()
}

fn merge_node_applications(applications: &mut Vec<Application>, discovery: &RawNodeDiscovery) {
    for package in &discovery.packages {
        let framework_signals: Vec<&RawTechnologySignal> = package
            .technologies
            .iter()
            .filter(|technology| technology.kind == "framework")
            .collect();
        let existing = applications
            .iter()
            .position(|application| application.application_dir == package.path);
        if existing.is_none() && framework_signals.is_empty() && !package.vite.standalone {
            continue;
        }

        let index = existing.unwrap_or_else(|| {
            applications.push(Application {
                application_dir: package.path.clone(),
                name: package.name.clone(),
                technologies: Vec::new(),
                entrypoint: None,
                dependencies: Vec::new(),
                build_scripts: Vec::new(),
                env_vars: Vec::new(),
                python: None,
                node: None,
                evidence: Vec::new(),
                diagnostics: Vec::new(),
            });
            applications.len() - 1
        });
        let application = &mut applications[index];
        application.node = Some(NodeInfo {
            requires_node: package.requires_node.clone(),
            version_pins: package
                .version_pins
                .iter()
                .map(|pin| VersionPin {
                    source: pin.source.clone(),
                    value: pin.value.clone(),
                })
                .collect(),
        });
        let had_primary = application
            .technologies
            .iter()
            .any(|technology| technology.role == "primary");
        if application.name.is_none()
            || (!had_primary && (!framework_signals.is_empty() || package.vite.standalone))
        {
            if let Some(name) = &package.name {
                application.name = Some(name.clone());
            }
        }
        for signal in &package.technologies {
            if signal.id == "inertia" {
                continue;
            }
            let name = match signal.id.as_str() {
                "solid-js" => "solid",
                other => other,
            };
            let kind = match signal.kind.as_str() {
                "ui" => "ui-framework",
                other => other,
            };
            let role = if signal.kind == "framework"
                || (signal.id == "vite"
                    && package.vite.standalone
                    && framework_signals.is_empty()
                    && !had_primary)
            {
                "primary"
            } else {
                "supporting"
            };
            merge_technology(
                &mut application.technologies,
                Technology {
                    name: name.to_string(),
                    kind: kind.to_string(),
                    role: role.to_string(),
                    confidence: "high".to_string(),
                    evidence: node_signal_evidence(package, signal),
                },
            );
        }

        if has_declared_dependency(application, "python", "cross-inertia")
            && package.inertia.corroborated
        {
            let mut evidence = package
                .technologies
                .iter()
                .find(|technology| technology.id == "inertia")
                .map(|technology| node_signal_evidence(package, technology))
                .unwrap_or_default();
            evidence.extend(
                application
                    .dependencies
                    .iter()
                    .filter(|dependencies| dependencies.ecosystem == "python")
                    .flat_map(|dependencies| &dependencies.declared)
                    .filter(|dependency| dependency.name == "cross-inertia")
                    .map(|dependency| Evidence {
                        kind: "dependency-declared".to_string(),
                        path: dependency.source.path.clone(),
                        span: dependency.source.span.clone(),
                        detail: format!("{} in `{}`", dependency.raw, dependency.group),
                    }),
            );
            merge_technology(
                &mut application.technologies,
                Technology {
                    name: "cross-inertia".to_string(),
                    kind: "integration".to_string(),
                    role: "supporting".to_string(),
                    confidence: "high".to_string(),
                    evidence,
                },
            );
        }

        application.dependencies.push(node_dependency_set(package));
        if let Some(command) = package.scripts.get("build") {
            application.build_scripts.push(BuildScript {
                name: "build".to_string(),
                command: command.clone(),
                package_manager: package
                    .package_manager
                    .as_ref()
                    .map(|manager| manager.name.clone()),
                argv: safe_argv(command),
                source: SourceRef {
                    path: package.manifest_path.clone(),
                    span: None,
                },
            });
        }
        if package.package_manager_candidates.len() > 1 {
            application.diagnostics.push(diag::kb308(
                &package.path,
                &package.package_manager_candidates,
            ));
        }

        application
            .technologies
            .sort_by(|a, b| (&a.role, &a.kind, &a.name).cmp(&(&b.role, &b.kind, &b.name)));
        application.dependencies.sort_by(|a, b| {
            (&a.ecosystem, &a.package_manager).cmp(&(&b.ecosystem, &b.package_manager))
        });
        application
            .build_scripts
            .sort_by(|a, b| a.name.cmp(&b.name));
        application.evidence = application
            .technologies
            .iter()
            .flat_map(|technology| technology.evidence.clone())
            .collect();
        application
            .evidence
            .sort_by(|a, b| (&a.path, &a.kind, &a.detail).cmp(&(&b.path, &b.kind, &b.detail)));
        application
            .evidence
            .dedup_by(|a, b| a.kind == b.kind && a.path == b.path && a.detail == b.detail);

        let primary: Vec<String> = application
            .technologies
            .iter()
            .filter(|technology| technology.role == "primary")
            .map(|technology| technology.name.clone())
            .collect();
        if primary.len() > 1 {
            application
                .diagnostics
                .push(diag::kb101(&application.application_dir, &primary));
        }
        dedup_sort_diags(&mut application.diagnostics);
    }
}

fn merge_technology(technologies: &mut Vec<Technology>, incoming: Technology) {
    if let Some(existing) = technologies
        .iter_mut()
        .find(|technology| technology.name == incoming.name && technology.kind == incoming.kind)
    {
        if incoming.role == "primary" {
            existing.role = "primary".to_string();
        }
        existing.evidence.extend(incoming.evidence);
        existing
            .evidence
            .sort_by(|a, b| (&a.path, &a.kind, &a.detail).cmp(&(&b.path, &b.kind, &b.detail)));
        existing
            .evidence
            .dedup_by(|a, b| a.kind == b.kind && a.path == b.path && a.detail == b.detail);
    } else {
        technologies.push(incoming);
    }
}

fn node_signal_evidence(package: &RawNodePackage, signal: &RawTechnologySignal) -> Vec<Evidence> {
    signal
        .evidence
        .iter()
        .map(|detail| {
            let (kind, path) = if let Some(path) = detail.strip_prefix("config:") {
                ("marker-file", path.to_string())
            } else if let Some(path) = detail.strip_prefix("marker:") {
                ("marker-file", path.to_string())
            } else if detail.starts_with("script:") {
                ("build-script", package.manifest_path.clone())
            } else if detail.contains("Dependencies:")
                || detail.starts_with("dependencies:")
                || detail.starts_with("dependency:")
            {
                ("dependency-declared", package.manifest_path.clone())
            } else if detail.contains('/') || detail.contains('.') {
                ("marker-file", detail.to_string())
            } else {
                ("marker-file", package.manifest_path.clone())
            };
            Evidence {
                kind: kind.to_string(),
                path,
                span: None,
                detail: detail.clone(),
            }
        })
        .collect()
}

fn node_dependency_set(package: &RawNodePackage) -> DependencySet {
    let mut declared = Vec::new();
    for (group, dependencies) in [
        ("dependencies", &package.dependencies),
        ("devDependencies", &package.dev_dependencies),
        ("optionalDependencies", &package.optional_dependencies),
    ] {
        for (name, specifier) in dependencies {
            declared.push(DeclaredDep {
                name: name.clone(),
                raw: format!("{name}@{specifier}"),
                specifier: specifier.clone(),
                extras: Vec::new(),
                markers: None,
                group: group.to_string(),
                source: SourceRef {
                    path: package.manifest_path.clone(),
                    span: None,
                },
            });
        }
    }
    declared.sort_by(|a, b| (&a.group, &a.name).cmp(&(&b.group, &b.name)));

    DependencySet {
        ecosystem: "node".to_string(),
        package_manager: package
            .package_manager
            .as_ref()
            .map(|manager| manager.name.clone()),
        manifests: vec![ManifestRef {
            path: package.manifest_path.clone(),
            kind: "package-json".to_string(),
        }],
        lockfiles: package
            .lockfiles
            .iter()
            .map(|path| LockfileRef {
                path: path.clone(),
                kind: node_lockfile_kind(path).to_string(),
                parsed: false,
            })
            .collect(),
        declared,
        resolved: Vec::new(),
    }
}

fn node_lockfile_kind(path: &str) -> &'static str {
    match path.rsplit('/').next().unwrap_or(path) {
        "package-lock.json" | "npm-shrinkwrap.json" => "npm",
        "pnpm-lock.yaml" => "pnpm",
        "yarn.lock" => "yarn",
        "bun.lock" | "bun.lockb" => "bun",
        _ => "node",
    }
}

fn has_declared_dependency(application: &Application, ecosystem: &str, name: &str) -> bool {
    application.dependencies.iter().any(|dependencies| {
        dependencies.ecosystem == ecosystem
            && dependencies
                .declared
                .iter()
                .any(|dependency| dependency.name == name)
    })
}

fn safe_argv(command: &str) -> Option<Vec<String>> {
    if command.is_empty()
        || command
            .chars()
            .any(|character| "|&;<>()$`\\\n\r\"'".contains(character))
    {
        return None;
    }
    let argv: Vec<String> = command
        .split_whitespace()
        .map(ToString::to_string)
        .collect();
    (!argv.is_empty()).then_some(argv)
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
            None if !fs.is_pending(pp_path) => diagnostics.push(diag::kb801(
                pp_path,
                "pyproject.toml is unreadable, non-UTF-8, or exceeds the 2 MiB parse cap",
            )),
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

    // Declared Python dependencies.
    let mut declared: Vec<DeclaredDep> = Vec::new();
    if let (Some(pp), Some(pp_path)) = (&parsed, &files.pyproject) {
        declared.extend(manifest::pyproject_deps(pp, pp_path));
    }
    manifest::requirements_project_deps(fs, &files.requirements, &mut declared);
    let mut inline_requires_python = None;
    for script in &files.inline_scripts {
        if let Some(source) = fs.read_str(script) {
            let (dependencies, requires_python) = manifest::inline_script_deps(&source, script);
            declared.extend(dependencies);
            if inline_requires_python.is_none() {
                inline_requires_python = requires_python;
            }
        }
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

    // Resolved Python dependencies from supported lockfiles.
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

    // Framework identity from dependencies, with group provenance.
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
    // Django marker-file identity.
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

    // Evident-install-path rule: which FastAPI declarations install?
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

    // Python version facts and KB700 conflicts.
    let requires_python = parsed
        .as_ref()
        .and_then(|pp| pp.project.as_ref())
        .and_then(|p| p.requires_python.clone())
        .or(inline_requires_python);
    let raw_version_pins = runtime::python_version_pins(fs, dir);
    if let Some(requirement) = requires_python
        .as_deref()
        .and_then(|value| value.parse::<pep440_rs::VersionSpecifiers>().ok())
    {
        for pin in &raw_version_pins {
            if let Ok(version) = pin
                .value
                .trim_start_matches(['v', 'V'])
                .parse::<pep440_rs::Version>()
            {
                if !requirement.contains(&version) {
                    diagnostics.push(diag::kb700(
                        &display_path,
                        &format!(
                            "{} pins {} but requires-python is {}",
                            pin.source,
                            pin.value,
                            requires_python.as_deref().unwrap_or("")
                        ),
                    ));
                }
            }
        }
    }
    let version_pins = raw_version_pins
        .into_iter()
        .map(|pin| VersionPin {
            source: pin.source,
            value: pin.value,
        })
        .collect();

    // ── FastAPI entrypoint resolution ──────────────────────────────────
    let tool_entrypoint = parsed
        .as_ref()
        .and_then(|pp| pp.tool.as_ref())
        .and_then(|t| t.fastapi.as_ref())
        .and_then(|f| f.entrypoint.clone());
    let should_resolve_entrypoint = !fs.is_virtual()
        || fastapi_declared
        || entrypoint_hint.is_some()
        || tool_entrypoint.is_some();
    if fastapi_declared {
        fs.enable_script_hints();
    }

    let mut resolution: Option<Resolution> = None;
    if should_resolve_entrypoint {
        if let Some(hint) = entrypoint_hint {
            allow_entrypoint_scripts(fs, dir, hint);
            match entrypoint::validate_entrypoint(fs, dir, hint, "hint") {
                Ok(res) => resolution = Some(res),
                Err(diags) => diagnostics.extend(diags),
            }
        }
    }
    if should_resolve_entrypoint && resolution.is_none() {
        if let Some(spec) = &tool_entrypoint {
            allow_entrypoint_scripts(fs, dir, spec);
            match entrypoint::validate_entrypoint(fs, dir, spec, "tool-fastapi") {
                Ok(res) => resolution = Some(res),
                Err(diags) => diagnostics.extend(diags),
            }
        }
    }
    let mut router_only = false;
    let mut import_seen = false;
    if should_resolve_entrypoint && resolution.is_none() {
        let scan = entrypoint::resolve_project(fs, dir);
        diagnostics.extend(scan.diagnostics);
        evidence.extend(scan.evidence);
        router_only = scan.router_only;
        import_seen = scan.fastapi_import_seen;
        resolution = scan.resolution;
    }

    // ── Internal FastAPI target construction and confidence ───────────
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
            "low" // convention-only
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
            evidence: res.evidence,
            diagnostics: res.diagnostics,
        });
    } else if fastapi_declared {
        // KB103: framework declared, but no app object was resolved.
        let kb103 = diag::kb103(&display_path, "fastapi");
        deploy_targets.push(DeployTarget {
            framework: "fastapi".to_string(),
            entrypoint: None,
            confidence: if dep_cap { "low" } else { "medium" }.to_string(),
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
        is_python_project: !is_ws_root
            || parsed
                .as_ref()
                .is_some_and(|project| project.project.is_some()),
        frameworks,
        deploy_targets,
        dependencies: Some(DependencySet {
            ecosystem: "python".to_string(),
            package_manager: (package_manager != "unknown").then(|| package_manager.to_string()),
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

fn allow_entrypoint_scripts(fs: &FileSet, project_dir: &str, spec: &str) {
    let Some((module, _)) = spec.split_once(':') else {
        return;
    };
    let module_path = module.replace('.', "/");
    for root in ["", "src"] {
        let base = match (
            project_dir.is_empty() || project_dir == ".",
            root.is_empty(),
        ) {
            (true, true) => String::new(),
            (true, false) => root.to_string(),
            (false, true) => project_dir.to_string(),
            (false, false) => format!("{project_dir}/{root}"),
        };
        let prefix = if base.is_empty() {
            String::new()
        } else {
            format!("{base}/")
        };
        fs.allow_script(format!("{prefix}{module_path}.py"));
        fs.allow_script(format!("{prefix}{module_path}/__init__.py"));
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
    } else if files.pyproject.is_some()
        || files
            .manifests
            .iter()
            .any(|manifest| manifest.kind == "inline-script")
    {
        "uv"
    } else if !files.requirements.is_empty() {
        "pip"
    } else {
        "unknown"
    }
}

/// Whether the scan origin is inside (or equal to) the project directory.
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

fn is_absolute_like(path: &str) -> bool {
    let normalized = path.replace('\\', "/");
    normalized.starts_with('/')
        || normalized.as_bytes().get(1) == Some(&b':')
            && normalized
                .as_bytes()
                .get(2)
                .is_some_and(|separator| *separator == b'/')
}

/// Dedup exact diagnostics, preserving distinct actionable messages that share
/// a code and location.
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
                d.message.clone(),
            )
        };
        key(a).cmp(&key(b))
    });
    diags.dedup_by(|a, b| {
        a.code == b.code
            && a.message == b.message
            && a.path == b.path
            && a.span.as_ref().map(|s| (s.start_line, s.start_col))
                == b.span.as_ref().map(|s| (s.start_line, s.start_col))
    });
}
