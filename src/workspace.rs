//! uv and Node workspace discovery, expansion, and upward path framing.

use std::path::{Path, PathBuf};

use crate::diag;
use crate::fileset::FileSet;
use crate::manifest::{parse_pyproject, UvWorkspace};
use crate::model::{Diagnostic, Workspace};
use crate::node;

/// Result of upward discovery: the directory the
/// walk should actually start from, plus the relative frames for the result.
pub struct EffectiveRoot {
    pub walk_root: PathBuf,
    /// upload_root relative to the scan root as given (e.g. "../..", or ".")
    pub upload_root: String,
    /// scan root relative to upload_root (e.g. "apps/api", or ".")
    pub scan_origin: String,
}

/// Walk real ancestors looking for a uv or Node workspace that contains the
/// scan root, including directories nested inside a declared member.
pub fn discover_upward(scan_root: &Path) -> EffectiveRoot {
    let same = EffectiveRoot {
        walk_root: scan_root.to_path_buf(),
        upload_root: ".".into(),
        scan_origin: ".".into(),
    };

    // If the scan root itself is a workspace root, nothing to discover.
    if let Some(ws) = read_workspace_table(&scan_root.join("pyproject.toml")) {
        let _ = ws;
        return same;
    }
    if read_node_workspace_patterns(scan_root).is_some() {
        return same;
    }

    let mut ups = 0usize;
    let mut current = scan_root.to_path_buf();
    while let Some(parent) = current.parent().map(Path::to_path_buf) {
        ups += 1;
        if ups > 16 {
            break;
        }
        let uv_workspace = read_workspace_table(&parent.join("pyproject.toml"));
        let node_patterns = read_node_workspace_patterns(&parent);
        if uv_workspace.is_some() || node_patterns.is_some() {
            let Ok(rel) = scan_root.strip_prefix(&parent) else {
                break;
            };
            let rel = rel
                .to_string_lossy()
                .replace(std::path::MAIN_SEPARATOR, "/");
            if uv_workspace
                .as_ref()
                .is_some_and(|workspace| member_globs_include(&parent, workspace, &rel))
                || node_patterns
                    .as_ref()
                    .is_some_and(|patterns| node_member_globs_include(&parent, patterns, &rel))
            {
                return EffectiveRoot {
                    walk_root: parent,
                    upload_root: vec![".."; ups].join("/"),
                    scan_origin: rel,
                };
            }
            break;
        }
        current = parent;
    }
    same
}

fn read_node_workspace_patterns(root: &Path) -> Option<Vec<String>> {
    let mut declared = false;
    let mut patterns = Vec::new();
    if let Ok(source) = std::fs::read_to_string(root.join("package.json")) {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&source) {
            if let Some(workspaces) = value.get("workspaces") {
                declared = true;
                let values = match workspaces {
                    serde_json::Value::Array(values) => Some(values),
                    serde_json::Value::Object(object) => {
                        object.get("packages").and_then(|value| value.as_array())
                    }
                    _ => None,
                };
                if let Some(values) = values {
                    patterns.extend(
                        values
                            .iter()
                            .filter_map(|value| value.as_str())
                            .map(str::to_string),
                    );
                }
            }
        }
    }
    if let Ok(source) = std::fs::read_to_string(root.join("pnpm-workspace.yaml")) {
        declared = true;
        let (pnpm_patterns, _) = node::parse_pnpm_workspace_yaml(&source);
        patterns.extend(pnpm_patterns);
    }
    if !declared {
        return None;
    }
    patterns.sort();
    patterns.dedup();
    Some(patterns)
}

fn node_member_globs_include(root: &Path, patterns: &[String], rel: &str) -> bool {
    manifest_ancestors(root, rel, "package.json")
        .iter()
        .any(|candidate| {
            let excluded = patterns
                .iter()
                .filter_map(|pattern| pattern.strip_prefix('!'))
                .any(|pattern| glob_matches(pattern, candidate));
            !excluded
                && patterns
                    .iter()
                    .filter(|pattern| !pattern.starts_with('!'))
                    .any(|pattern| glob_matches(pattern, candidate))
        })
}

fn read_workspace_table(pyproject: &Path) -> Option<UvWorkspace> {
    let source = std::fs::read_to_string(pyproject).ok()?;
    let pp = parse_pyproject(&source).ok()?;
    pp.tool.and_then(|t| t.uv).and_then(|u| u.workspace)
}

fn member_globs_include(root: &Path, ws: &UvWorkspace, rel: &str) -> bool {
    manifest_ancestors(root, rel, "pyproject.toml")
        .iter()
        .any(|candidate| {
            let excluded = ws
                .exclude
                .iter()
                .flatten()
                .any(|pattern| glob_matches(pattern, candidate));
            !excluded
                && ws
                    .members
                    .iter()
                    .flatten()
                    .any(|pattern| glob_matches(pattern, candidate))
        })
}

fn manifest_ancestors(root: &Path, rel: &str, manifest: &str) -> Vec<String> {
    let mut matches = Vec::new();
    let mut candidate = rel;
    loop {
        if root.join(candidate).join(manifest).is_file() {
            matches.push(candidate.to_string());
        }
        let Some((parent, _)) = candidate.rsplit_once('/') else {
            return matches;
        };
        candidate = parent;
    }
}

fn glob_matches(pattern: &str, rel: &str) -> bool {
    let pattern = pattern.trim_start_matches("./");
    let options = glob::MatchOptions {
        // `*` must not cross `/` (uv's glob semantics): `apps/*` matches
        // apps/api but not apps/api/app; `**` still recurses.
        require_literal_separator: true,
        ..glob::MatchOptions::default()
    };
    glob::Pattern::new(pattern).is_ok_and(|p| p.matches_with(rel, options))
}

pub struct WorkspaceInfo {
    pub workspace: Workspace,
    pub diagnostics: Vec<Diagnostic>,
}

/// Expand a workspace declared at the fileset root (path "."): members and
/// exclude are root-relative globs, exclude wins, hidden entries are skipped,
/// and the root is always a member.
pub fn expand_workspace(
    fs: &FileSet,
    ws: &UvWorkspace,
    root_has_project_table: bool,
) -> WorkspaceInfo {
    let mut diagnostics = Vec::new();
    let mut members: Vec<String> = vec![".".to_string()];

    let pyproject_dirs = fs.dirs_with("pyproject.toml");
    let all_dirs = candidate_dirs(fs);

    for pattern in ws.members.iter().flatten() {
        let mut matched_any = false;
        for dir in &all_dirs {
            if dir.is_empty()
                || dir.split('/').any(|part| part.starts_with('.'))
                || !glob_matches(pattern, dir)
            {
                continue;
            }
            if ws.exclude.iter().flatten().any(|ex| glob_matches(ex, dir)) {
                continue; // exclude wins
            }
            matched_any = true;
            if pyproject_dirs.contains(dir) {
                if !members.contains(dir) {
                    members.push(dir.clone());
                }
            } else {
                diagnostics.push(diag::kb400(".", dir));
            }
        }
        if !matched_any {
            diagnostics.push(diag::kb402(".", pattern));
        }
    }

    // Nested uv workspace tables in members produce KB401.
    for member in &members {
        if member == "." {
            continue;
        }
        let pp_path = format!("{member}/pyproject.toml");
        if let Some(source) = fs.read_str(&pp_path) {
            if let Ok(pp) = parse_pyproject(&source) {
                if pp
                    .tool
                    .and_then(|t| t.uv)
                    .and_then(|u| u.workspace)
                    .is_some()
                {
                    diagnostics.push(diag::kb401(member));
                }
            }
        }
    }

    members.sort_by(|a, b| {
        // root first, then declaration-agnostic byte order (M1 approximation
        // of declaration order; glob expansion above is already byte-ordered)
        (a != ".").cmp(&(b != ".")).then(a.cmp(b))
    });

    WorkspaceInfo {
        workspace: Workspace {
            kind: "uv".into(),
            path: ".".into(),
            virtual_root: !root_has_project_table,
            members,
        },
        diagnostics,
    }
}

/// All directories that appear in the fileset (parents of every file).
fn candidate_dirs(fs: &FileSet) -> Vec<String> {
    let mut dirs = std::collections::BTreeSet::new();
    for path in fs.files.keys() {
        let mut current = path.as_str();
        while let Some((parent, _)) = current.rsplit_once('/') {
            dirs.insert(parent.to_string());
            current = parent;
        }
    }
    dirs.into_iter().collect()
}
