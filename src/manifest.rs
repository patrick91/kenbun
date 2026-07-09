//! Manifest parsing — spec §6.2 (Layer 1) and §8 (dependency metadata).

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::fileset::FileSet;
use crate::model::{DeclaredDep, LockfileRef, ManifestRef, ResolvedDep, SourceRef};
use crate::norm::{normalize_name, split_requirement};

// ── pyproject.toml ─────────────────────────────────────────────────────────

#[derive(Deserialize, Default)]
pub struct PyProject {
    pub project: Option<ProjectTable>,
    #[serde(rename = "dependency-groups")]
    pub dependency_groups: Option<toml::Table>,
    pub tool: Option<ToolTable>,
    #[serde(rename = "build-system")]
    pub build_system: Option<BuildSystem>,
}

#[derive(Deserialize, Default)]
pub struct BuildSystem {
    #[serde(rename = "build-backend")]
    pub build_backend: Option<String>,
}

#[derive(Deserialize, Default)]
pub struct ProjectTable {
    pub name: Option<String>,
    pub dependencies: Option<Vec<String>>,
    #[serde(rename = "optional-dependencies")]
    pub optional_dependencies: Option<BTreeMap<String, Vec<String>>>,
    #[serde(rename = "requires-python")]
    pub requires_python: Option<String>,
}

#[derive(Deserialize, Default)]
pub struct ToolTable {
    pub uv: Option<UvTable>,
    pub poetry: Option<toml::Table>,
    pub pdm: Option<toml::Value>,
    pub fastapi: Option<FastapiTable>,
}

#[derive(Deserialize, Default)]
pub struct UvTable {
    pub workspace: Option<UvWorkspace>,
    #[serde(rename = "dev-dependencies")]
    pub dev_dependencies: Option<Vec<String>>,
    #[allow(dead_code)] // M2: path-dep linking + env-var aggregation
    pub sources: Option<toml::Table>,
    #[serde(rename = "default-groups")]
    #[allow(dead_code)] // M2: literal default-groups in the KB301 rule
    pub default_groups: Option<toml::Value>,
}

#[derive(Deserialize, Default, Clone)]
pub struct UvWorkspace {
    pub members: Option<Vec<String>>,
    pub exclude: Option<Vec<String>>,
}

#[derive(Deserialize, Default)]
pub struct FastapiTable {
    pub entrypoint: Option<String>,
}

pub fn parse_pyproject(source: &str) -> Result<PyProject, String> {
    toml::from_str(source).map_err(|e| e.to_string())
}

// ── declared dependencies ──────────────────────────────────────────────────

fn declared(raw: &str, group: &str, path: &str) -> Option<DeclaredDep> {
    let (name, extras, rest) = split_requirement(raw)?;
    let (specifier, markers) = match rest.split_once(';') {
        Some((spec, marker)) => (spec.trim().to_string(), Some(marker.trim().to_string())),
        None => (rest, None),
    };
    Some(DeclaredDep {
        name,
        raw: raw.trim().to_string(),
        specifier,
        extras,
        markers,
        group: group.to_string(),
        source: SourceRef {
            path: path.to_string(),
            span: None,
        },
    })
}

/// All declared deps in a parsed pyproject, with group provenance (§8).
pub fn pyproject_deps(pp: &PyProject, path: &str) -> Vec<DeclaredDep> {
    let mut out = Vec::new();

    if let Some(project) = &pp.project {
        for raw in project.dependencies.iter().flatten() {
            out.extend(declared(raw, "project", path));
        }
        for (extra, reqs) in project.optional_dependencies.iter().flatten() {
            for raw in reqs {
                out.extend(declared(raw, &format!("optional:{extra}"), path));
            }
        }
    }

    // PEP 735 dependency-groups: strings or {include-group = "..."} tables.
    if let Some(groups) = &pp.dependency_groups {
        let expand = |group_name: &str, out: &mut Vec<DeclaredDep>| {
            let mut stack = vec![group_name.to_string()];
            let mut seen = std::collections::BTreeSet::new();
            while let Some(g) = stack.pop() {
                if !seen.insert(g.clone()) {
                    continue; // cycle guard (§6.2)
                }
                let Some(toml::Value::Array(items)) = groups.get(&g) else {
                    continue;
                };
                for item in items {
                    match item {
                        toml::Value::String(raw) => {
                            out.extend(declared(raw, &format!("group:{group_name}"), path));
                        }
                        toml::Value::Table(t) => {
                            if let Some(toml::Value::String(inc)) = t.get("include-group") {
                                stack.push(normalize_name(inc));
                            }
                        }
                        _ => {}
                    }
                }
            }
        };
        let names: Vec<String> = groups.keys().cloned().collect();
        for name in names {
            expand(&name, &mut out);
        }
    }

    if let Some(dev) = pp
        .tool
        .as_ref()
        .and_then(|t| t.uv.as_ref())
        .and_then(|u| u.dev_dependencies.as_ref())
    {
        for raw in dev {
            out.extend(declared(raw, "dev", path));
        }
    }

    // Poetry tables: keys are names; `python` is a version constraint (§6.2).
    if let Some(poetry) = pp.tool.as_ref().and_then(|t| t.poetry.as_ref()) {
        let poetry_keys = |table: &toml::Value, group: &str, out: &mut Vec<DeclaredDep>| {
            if let toml::Value::Table(t) = table {
                for key in t.keys() {
                    if normalize_name(key) == "python" {
                        continue;
                    }
                    out.extend(declared(key, group, path));
                }
            }
        };
        if let Some(deps) = poetry.get("dependencies") {
            poetry_keys(deps, "poetry:main", &mut out);
        }
        if let Some(deps) = poetry.get("dev-dependencies") {
            poetry_keys(deps, "poetry:dev", &mut out);
        }
        if let Some(toml::Value::Table(groups)) = poetry.get("group") {
            for (gname, gval) in groups {
                if let Some(deps) = gval.get("dependencies") {
                    poetry_keys(deps, &format!("poetry:group:{gname}"), &mut out);
                }
            }
        }
    }

    out
}

/// requirements.txt with recursive -r includes (§6.2; depth ≤ 5).
pub fn requirements_deps(fs: &FileSet, rel_path: &str, depth: u8, out: &mut Vec<DeclaredDep>) {
    if depth > 5 {
        return;
    }
    let Some(source) = fs.read_str(rel_path) else {
        return;
    };
    let dir = rel_path.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
    let group = if is_dev_requirements(rel_path) {
        format!("group:{rel_path}")
    } else {
        "project".to_string()
    };

    // join continuation lines
    let joined = source.replace("\\\n", " ");
    for line in joined.lines() {
        let line = line.split(" #").next().unwrap_or(line).trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(included) = line
            .strip_prefix("-r ")
            .or_else(|| line.strip_prefix("--requirement "))
        {
            let included = included.trim();
            let joined_path = if dir.is_empty() {
                included.to_string()
            } else {
                format!("{dir}/{included}")
            };
            requirements_deps(fs, &normalize_rel(&joined_path), depth + 1, out);
            continue;
        }
        if line.starts_with('-') {
            // -e, -c, --hash, --index-url…: name only via #egg= fragment
            if let Some(egg) = line.split("#egg=").nth(1) {
                let name = egg.split(&['&', ' '][..]).next().unwrap_or(egg);
                out.extend(declared(name, &group, rel_path));
            }
            continue;
        }
        out.extend(declared(line, &group, rel_path));
    }
}

fn is_dev_requirements(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    ["dev", "test", "lint", "doc"]
        .iter()
        .any(|k| lower.contains(k))
}

fn normalize_rel(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            p => parts.push(p),
        }
    }
    parts.join("/")
}

// ── lockfiles ──────────────────────────────────────────────────────────────

/// uv.lock / pylock.toml resolved packages (§8): [[package]] name + version.
pub fn parse_lock_resolved(source: &str, lock_rel: &str, kind: &str) -> Vec<ResolvedDep> {
    let Ok(value) = source.parse::<toml::Table>() else {
        return Vec::new();
    };
    let key = if kind == "pylock" {
        "packages"
    } else {
        "package"
    };
    let Some(toml::Value::Array(packages)) = value.get(key) else {
        return Vec::new();
    };
    let mut out: Vec<ResolvedDep> = packages
        .iter()
        .filter_map(|p| {
            let name = p.get("name")?.as_str()?;
            let version = p.get("version").and_then(|v| v.as_str()).unwrap_or("");
            Some(ResolvedDep {
                name: normalize_name(name),
                version: version.to_string(),
                source: lock_rel.to_string(),
                marker: p
                    .get("resolution-markers")
                    .and_then(|m| m.as_array())
                    .and_then(|a| a.first())
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
            })
        })
        .collect();
    out.sort_by(|a, b| (&a.name, &a.version).cmp(&(&b.name, &b.version)));
    out
}

/// Pipfile [packages]/[dev-packages]: keys are package names (§6.2).
pub fn pipfile_deps(source: &str, path: &str) -> Vec<DeclaredDep> {
    let Ok(value) = source.parse::<toml::Table>() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (table, group) in [("packages", "project"), ("dev-packages", "dev")] {
        if let Some(toml::Value::Table(deps)) = value.get(table) {
            for key in deps.keys() {
                out.extend(declared(key, group, path));
            }
        }
    }
    out
}

/// setup.py string scan (§13: no execution — quoted strings only). Returns
/// normalized names of known frameworks mentioned in string literals.
pub fn setup_py_framework_mentions(source: &str) -> Vec<String> {
    let mut found = std::collections::BTreeSet::new();
    for quote in ['"', '\''] {
        for chunk in source.split(quote).skip(1).step_by(2) {
            if let Some((name, _, _)) = split_requirement(chunk) {
                if framework_for(&name).is_some() {
                    found.insert(name);
                }
            }
        }
    }
    found.into_iter().collect()
}

/// Names recorded in uv.lock (for the conservative KB302 check and Layer 1).
#[allow(dead_code)] // M2: KB302 drift check
pub fn lock_package_names(resolved: &[ResolvedDep]) -> std::collections::BTreeSet<String> {
    resolved.iter().map(|r| r.name.clone()).collect()
}

// ── manifest/lockfile discovery per project dir ────────────────────────────

pub struct ProjectFiles {
    pub pyproject: Option<String>,
    pub manifests: Vec<ManifestRef>,
    pub lockfiles: Vec<LockfileRef>,
    pub requirements: Vec<String>,
}

pub fn project_files(fs: &FileSet, dir: &str) -> ProjectFiles {
    let join = |name: &str| {
        if dir.is_empty() {
            name.to_string()
        } else {
            format!("{dir}/{name}")
        }
    };
    let mut manifests = Vec::new();
    let mut lockfiles = Vec::new();
    let mut requirements = Vec::new();
    let mut pyproject = None;

    let pp = join("pyproject.toml");
    if fs.contains(&pp) {
        manifests.push(ManifestRef {
            path: pp.clone(),
            kind: "pyproject".into(),
        });
        pyproject = Some(pp);
    }
    for (file, kind) in [
        ("setup.py", "setup-py"),
        ("setup.cfg", "setup-cfg"),
        ("Pipfile", "pipfile"),
    ] {
        let p = join(file);
        if fs.contains(&p) {
            manifests.push(ManifestRef {
                path: p,
                kind: kind.into(),
            });
        }
    }
    // requirements*.txt at the project root plus requirements/*.txt (§6.2)
    for path in fs.under(dir) {
        let rel_in_dir = if dir.is_empty() {
            path
        } else {
            &path[dir.len() + 1..]
        };
        let is_root_req = !rel_in_dir.contains('/')
            && rel_in_dir.starts_with("requirements")
            && rel_in_dir.ends_with(".txt");
        let is_req_dir = rel_in_dir.starts_with("requirements/")
            && rel_in_dir.ends_with(".txt")
            && rel_in_dir.matches('/').count() == 1;
        if is_root_req || is_req_dir {
            manifests.push(ManifestRef {
                path: path.to_string(),
                kind: "requirements".into(),
            });
            requirements.push(path.to_string());
        }
    }
    for (file, kind) in [
        ("uv.lock", "uv"),
        ("pylock.toml", "pylock"),
        ("poetry.lock", "poetry"),
        ("pdm.lock", "pdm"),
        ("Pipfile.lock", "pipenv"),
    ] {
        let p = join(file);
        if fs.contains(&p) {
            lockfiles.push(LockfileRef {
                path: p,
                kind: kind.into(),
                parsed: matches!(kind, "uv" | "pylock"),
            });
        }
    }
    manifests.sort_by(|a, b| a.path.cmp(&b.path));
    requirements.sort();

    ProjectFiles {
        pyproject,
        manifests,
        lockfiles,
        requirements,
    }
}

/// Framework identity specs (§6.2, §12): fastapi resolved; django/flask identity-only.
pub fn framework_for(name: &str) -> Option<&'static str> {
    match name {
        "fastapi" | "fastapi-slim" => Some("fastapi"),
        "django" => Some("django"),
        "flask" => Some("flask"),
        _ => None,
    }
}
