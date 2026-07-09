//! uv workspace discovery — spec §7.

use std::path::{Path, PathBuf};

use crate::diag;
use crate::fileset::FileSet;
use crate::manifest::{parse_pyproject, UvWorkspace};
use crate::model::{Diagnostic, Workspace};

/// Result of upward discovery (fs mode only, spec §7): the directory the
/// walk should actually start from, plus the relative frames for the result.
pub struct EffectiveRoot {
    pub walk_root: PathBuf,
    /// upload_root relative to the scan root as given (e.g. "../..", or ".")
    pub upload_root: String,
    /// scan root relative to upload_root (e.g. "apps/api", or ".")
    pub scan_origin: String,
}

/// Walk real ancestors looking for a uv workspace root that includes the
/// scan root as a member (mirrors uv's Workspace::discover).
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

    let mut ups = 0usize;
    let mut current = scan_root.to_path_buf();
    while let Some(parent) = current.parent().map(Path::to_path_buf) {
        ups += 1;
        if ups > 16 {
            break;
        }
        if let Some(ws) = read_workspace_table(&parent.join("pyproject.toml")) {
            let Ok(rel) = scan_root.strip_prefix(&parent) else {
                break;
            };
            let rel = rel
                .to_string_lossy()
                .replace(std::path::MAIN_SEPARATOR, "/");
            if member_globs_include(&ws, &rel) {
                return EffectiveRoot {
                    walk_root: parent,
                    upload_root: vec![".."; ups].join("/"),
                    scan_origin: rel,
                };
            }
            break; // nearest workspace root doesn't include us — stop like uv
        }
        current = parent;
    }
    same
}

fn read_workspace_table(pyproject: &Path) -> Option<UvWorkspace> {
    let source = std::fs::read_to_string(pyproject).ok()?;
    let pp = parse_pyproject(&source).ok()?;
    pp.tool.and_then(|t| t.uv).and_then(|u| u.workspace)
}

fn member_globs_include(ws: &UvWorkspace, rel: &str) -> bool {
    let excluded = ws
        .exclude
        .iter()
        .flatten()
        .any(|pattern| glob_matches(pattern, rel));
    if excluded {
        return false; // exclude wins (§7)
    }
    ws.members
        .iter()
        .flatten()
        .any(|pattern| glob_matches(pattern, rel))
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
/// exclude are root-relative globs, exclude wins, hidden entries skipped,
/// the root is always a member (§7).
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

    // Nested workspace tables in members are an error uv-side (§7 / KB401).
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
