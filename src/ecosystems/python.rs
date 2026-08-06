//! Python project discovery and analysis.
//!
//! This module owns Python-specific candidate types and facts. It deliberately
//! stops before constructing public `Application` objects so the assembler can
//! reconcile Python and JavaScript/TypeScript contributions at the same path.

use std::collections::BTreeSet;

mod frameworks;
pub(crate) mod manifest;
mod norm;

use super::runtime;
use crate::diag;
use crate::fileset::FileSet;
use crate::model::{
    DeclaredDep, DependencySet, Diagnostic, EnvVar, Evidence, PythonInfo, VersionPin,
};
use manifest::PyProject;

#[derive(Clone, Debug)]
pub(crate) struct Project {
    pub path: String,
    pub name: Option<String>,
    pub is_python_project: bool,
    pub frameworks: Vec<String>,
    pub deploy_targets: Vec<frameworks::DeployTarget>,
    pub dependencies: Option<DependencySet>,
    pub env_vars: Vec<EnvVar>,
    pub python: PythonInfo,
    pub evidence: Vec<Evidence>,
    pub diagnostics: Vec<Diagnostic>,
}

/// Discover and analyze every Python project in the shared repository
/// inventory. Projects without a primary Python framework remain available to
/// enrich a JavaScript/TypeScript application at the same directory.
pub(crate) fn discover(
    fs: &FileSet,
    hint_dir: Option<&str>,
    scan_origin: &str,
    entrypoint_hint: Option<&str>,
) -> Vec<Project> {
    let pyproject_dirs = fs.dirs_with("pyproject.toml");
    let mut project_dirs: BTreeSet<String> = pyproject_dirs.iter().cloned().collect();

    // requirements/Pipfile/setup.py projects: not nested under a pyproject project
    for marker in ["requirements.txt", "Pipfile", "setup.py"] {
        for dir in fs.dirs_with(marker) {
            let nested = pyproject_dirs.iter().any(|project| {
                project == &dir || (!project.is_empty() && dir.starts_with(&format!("{project}/")))
            });
            if !nested {
                project_dirs.insert(dir);
            }
        }
    }

    // bare-scripts root project: *.py at root, no manifests anywhere
    if project_dirs.is_empty() && fs.files.keys().any(|path| path.ends_with(".py")) {
        project_dirs.insert(String::new());
    }

    project_dirs
        .iter()
        .map(|dir| {
            let hint = if hint_dir == Some(dir.as_str())
                || (hint_dir.is_none() && origin_matches(scan_origin, dir))
            {
                entrypoint_hint
            } else {
                None
            };
            analyze_project(fs, dir, hint)
        })
        .collect()
}

fn analyze_project(fs: &FileSet, dir: &str, entrypoint_hint: Option<&str>) -> Project {
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
        match fs
            .read_str(pp_path)
            .map(|source| manifest::parse_pyproject(&source))
        {
            Some(Ok(pyproject)) => parsed = Some(pyproject),
            Some(Err(error)) => diagnostics.push(diag::kb201(pp_path, &error)),
            None if !fs.is_pending(pp_path) => diagnostics.push(diag::kb801(
                pp_path,
                &format!(
                    "pyproject.toml is unreadable, non-UTF-8, or exceeds the {}-byte parse cap",
                    fs.max_file_bytes()
                ),
            )),
            None => {}
        }
    }
    let is_workspace_root = parsed
        .as_ref()
        .and_then(|pyproject| pyproject.tool.as_ref())
        .and_then(|tool| tool.uv.as_ref())
        .is_some_and(|uv| uv.workspace.is_some());
    if let (Some(path), Some(pyproject)) = (&files.pyproject, &parsed) {
        if pyproject.project.is_none() && !is_workspace_root {
            diagnostics.push(diag::kb202(path));
        }
    }

    let mut declared: Vec<DeclaredDep> = Vec::new();
    if let (Some(pyproject), Some(path)) = (&parsed, &files.pyproject) {
        declared.extend(manifest::pyproject_deps(pyproject, path));
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
    let pipfile_path = join(dir, "Pipfile");
    if fs.contains(&pipfile_path) {
        if let Some(source) = fs.read_str(&pipfile_path) {
            declared.extend(manifest::pipfile_deps(&source, &pipfile_path));
        }
    }
    declared.sort_by(|a, b| (&a.name, &a.source.path).cmp(&(&b.name, &b.source.path)));

    let has_project_dependencies = parsed
        .as_ref()
        .and_then(|pyproject| pyproject.project.as_ref())
        .and_then(|project| project.dependencies.as_ref())
        .is_some_and(|dependencies| !dependencies.is_empty());
    let has_root_requirements = files
        .requirements
        .iter()
        .any(|path| path.rsplit('/').next() == Some("requirements.txt"));
    if has_project_dependencies && has_root_requirements {
        diagnostics.push(diag::kb300(&display_path));
    }
    let has_poetry_table = parsed
        .as_ref()
        .and_then(|pyproject| pyproject.tool.as_ref())
        .is_some_and(|tool| tool.poetry.is_some());
    let has_pdm_table = parsed
        .as_ref()
        .and_then(|pyproject| pyproject.tool.as_ref())
        .is_some_and(|tool| tool.pdm.is_some());
    let build_backend = parsed
        .as_ref()
        .and_then(|pyproject| pyproject.build_system.as_ref())
        .and_then(|build_system| build_system.build_backend.clone())
        .unwrap_or_default();
    let package_manager =
        detect_package_manager(&files, has_poetry_table, has_pdm_table, &build_backend);
    if matches!(package_manager, "poetry" | "pipenv" | "pdm") {
        diagnostics.push(diag::kb306(&display_path, package_manager));
    }

    let framework_detection = frameworks::detect(
        fs,
        dir,
        &display_path,
        &declared,
        parsed.as_ref(),
        package_manager,
        entrypoint_hint,
    );
    let frameworks::Detection {
        frameworks,
        deploy_targets,
        evidence: framework_evidence,
        diagnostics: framework_diagnostics,
    } = framework_detection;
    evidence.extend(framework_evidence);
    diagnostics.extend(framework_diagnostics);

    let requires_python = parsed
        .as_ref()
        .and_then(|pyproject| pyproject.project.as_ref())
        .and_then(|project| project.requires_python.clone())
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

    let name = parsed
        .as_ref()
        .and_then(|pyproject| pyproject.project.as_ref())
        .and_then(|project| project.name.clone())
        .or_else(|| {
            parsed
                .as_ref()
                .and_then(|pyproject| pyproject.tool.as_ref())
                .and_then(|tool| tool.poetry.as_ref())
                .and_then(|poetry| poetry.get("name"))
                .and_then(|name| name.as_str())
                .map(str::to_string)
        });

    evidence.sort_by(|a, b| (&a.path, &a.kind).cmp(&(&b.path, &b.kind)));
    diag::sort_and_dedup(&mut diagnostics);

    Project {
        path: display_path,
        name,
        is_python_project: !is_workspace_root
            || parsed
                .as_ref()
                .is_some_and(|pyproject| pyproject.project.is_some()),
        frameworks,
        deploy_targets,
        dependencies: Some(DependencySet {
            ecosystem: "python".to_string(),
            package_manager: (package_manager != "unknown").then(|| package_manager.to_string()),
            package_manager_version: None,
            manifests: files.manifests,
            declared,
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
    if has_poetry_table || build_backend.starts_with("poetry") {
        "poetry"
    } else if has_pdm_table || build_backend.starts_with("pdm") {
        "pdm"
    } else if files
        .manifests
        .iter()
        .any(|manifest| manifest.kind == "pipfile")
    {
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

fn origin_matches(origin: &str, project_dir: &str) -> bool {
    let origin = if origin == "." { "" } else { origin };
    let project = if project_dir == "." { "" } else { project_dir };
    if project.is_empty() {
        return origin.is_empty();
    }
    origin == project || origin.starts_with(&format!("{project}/"))
}

fn join(directory: &str, name: &str) -> String {
    if directory.is_empty() || directory == "." {
        name.to_string()
    } else {
        format!("{directory}/{name}")
    }
}
