//! Deterministic scan orchestration for local and virtual repository inputs.

use std::path::Path;

use crate::diag;
use crate::ecosystems::{assembly, node, python, workspace};
use crate::fileset::{self, FileSet};
use crate::model::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// The ecosystem detectors enabled for one analysis.
pub(crate) struct EcosystemSelection {
    pub python: bool,
    pub node: bool,
}

impl Default for EcosystemSelection {
    fn default() -> Self {
        Self {
            python: true,
            node: true,
        }
    }
}

pub struct ScanOptions {
    pub application_dir: Option<String>,
    pub entrypoint: Option<String>,
    pub ecosystems: EcosystemSelection,
    pub max_files: Option<u64>,
    pub follow_symlinks: bool,
    pub extra_ignore_files: Vec<String>,
    pub max_depth: Option<u64>,
}

pub fn scan(root: &Path, opts: &ScanOptions) -> ScanResult {
    let effective = workspace::discover_upward(root, opts.ecosystems.python, opts.ecosystems.node);
    let fs = fileset::walk_fs(
        &effective.walk_root,
        opts.max_files,
        opts.follow_symlinks,
        &opts.extra_ignore_files,
        opts.max_depth,
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

pub fn analyze(
    fs: &FileSet,
    inventory_complete: bool,
    ecosystems: EcosystemSelection,
) -> ScanResult {
    let opts = ScanOptions {
        application_dir: None,
        entrypoint: None,
        ecosystems,
        max_files: None,
        follow_symlinks: false,
        extra_ignore_files: Vec::new(),
        max_depth: None,
    };
    if fs.has_ignore_requests() {
        return finish_virtual_result(
            ScanResult {
                schema_version: SCHEMA_VERSION,
                root: ".".to_string(),
                upload_root: ".".to_string(),
                scan_origin: ".".to_string(),
                status: "complete".to_string(),
                completeness: "complete".to_string(),
                file_requests: Vec::new(),
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
    result.file_requests = fs.requests();
    result.status = if result.file_requests.is_empty() {
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
        || !result.file_requests.is_empty()
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
    let mut scan_diagnostics: Vec<Diagnostic> = Vec::new();
    for issue in &fs.issues {
        let diagnostic = if issue.message.contains("scan root") {
            diag::kb800(&issue.path, &issue.message)
        } else {
            diag::kb801(&issue.path, &issue.message)
        };
        scan_diagnostics.push(diagnostic);
    }
    if fs.truncated {
        scan_diagnostics.push(diag::kb802(opts.max_files.unwrap_or(0)));
    }

    let node_discovery = if opts.ecosystems.node {
        node::discover(fs)
    } else {
        node::RawNodeDiscovery::default()
    };
    scan_diagnostics.extend(
        node_discovery
            .parse_errors
            .iter()
            .map(|error| diag::kb203(&error.path, &error.message)),
    );

    let workspace_info = workspace::discover_at_root(
        fs,
        &node_discovery,
        opts.ecosystems.python,
        opts.ecosystems.node,
    );
    scan_diagnostics.extend(workspace_info.diagnostics);
    let mut workspace = workspace_info.workspace;
    let hint_dir = validate_application_dir(fs, opts, &scan_origin, &mut scan_diagnostics);
    let projects = if opts.ecosystems.python {
        python::discover(
            fs,
            hint_dir.as_deref(),
            &scan_origin,
            opts.entrypoint.as_deref(),
        )
    } else {
        Vec::new()
    };
    let applications = assembly::applications(&projects, &node_discovery);

    if let Some(workspace) = &mut workspace {
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
            scan_diagnostics.push(diag::kb502(
                opts.application_dir.as_deref().unwrap_or(display),
            ));
        }
    }

    let mut diagnostics = scan_diagnostics;
    for project in &projects {
        diagnostics.extend(project.diagnostics.iter().cloned());
        for target in &project.deploy_targets {
            diagnostics.extend(target.diagnostics.iter().cloned());
        }
    }
    for application in &applications {
        diagnostics.extend(application.diagnostics.iter().cloned());
    }
    diag::sort_and_dedup(&mut diagnostics);

    if applications.is_empty() {
        diagnostics.push(
            if projects.is_empty() && node_discovery.packages.is_empty() {
                diag::kb100()
            } else {
                diag::kb102()
            },
        );
        diag::sort_and_dedup(&mut diagnostics);
    }

    ScanResult {
        schema_version: SCHEMA_VERSION,
        root,
        upload_root,
        scan_origin,
        status: "complete".to_string(),
        completeness: "complete".to_string(),
        file_requests: Vec::new(),
        workspace,
        applications,
        diagnostics,
    }
}

fn validate_application_dir(
    fs: &FileSet,
    opts: &ScanOptions,
    scan_origin: &str,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<String> {
    let raw = opts.application_dir.as_ref()?;
    let normalized = normalize_rel(raw);
    if is_absolute_like(raw) || normalized.starts_with("..") {
        diagnostics.push(diag::kb501(raw));
        return None;
    }

    let scan_origin = if scan_origin == "." {
        String::new()
    } else {
        scan_origin.to_string()
    };
    let project = if normalized == "." {
        scan_origin
    } else if scan_origin.is_empty() {
        normalized
    } else {
        format!("{scan_origin}/{normalized}")
    };
    let exists = fs
        .files
        .keys()
        .any(|path| project.is_empty() || path.starts_with(&format!("{project}/")));
    if !exists {
        diagnostics.push(diag::kb500(raw));
        None
    } else {
        Some(project)
    }
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
            part => parts.push(part),
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
