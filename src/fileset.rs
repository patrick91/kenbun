//! Deterministic filesystem indexing and bounded file reads.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Per-file parse cap: larger files are skipped as unavailable.
pub const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;

const UNCONDITIONAL_EXCLUDES: &[&str] = &[
    ".git",
    ".hg",
    ".venv",
    "venv",
    ".tox",
    ".nox",
    "node_modules",
    "__pycache__",
    "site-packages",
    ".eggs",
    ".mypy_cache",
    ".ruff_cache",
    ".pytest_cache",
];

/// `env`/`build`/`dist` are only excluded when they look like venvs or
/// build output; real source directories may use these names.
const CONDITIONAL_EXCLUDES: &[&str] = &["env", "build", "dist"];
const VENV_BUILD_MARKERS: &[&str] = &["pyvenv.cfg", "bin/activate", "PKG-INFO"];

pub struct FileSet {
    pub root: PathBuf,
    /// Relative `/`-separated path to size; BTreeMap preserves byte ordering.
    pub files: BTreeMap<String, u64>,
    pub truncated: bool,
}

impl FileSet {
    pub fn contains(&self, rel: &str) -> bool {
        self.files.contains_key(rel)
    }

    /// Files directly or transitively under a directory (`""` = root).
    pub fn under<'a>(&'a self, dir: &'a str) -> impl Iterator<Item = &'a str> + 'a {
        let prefix = if dir.is_empty() {
            String::new()
        } else {
            format!("{dir}/")
        };
        self.files
            .range(prefix.clone()..)
            .take_while(move |(p, _)| p.starts_with(&prefix))
            .map(|(p, _)| p.as_str())
    }

    /// Directories (relative paths) that contain the given file name.
    pub fn dirs_with(&self, file_name: &str) -> Vec<String> {
        let suffix = format!("/{file_name}");
        let mut dirs: Vec<String> = self
            .files
            .keys()
            .filter_map(|p| {
                if p == file_name {
                    Some(String::new())
                } else {
                    p.strip_suffix(&suffix).map(str::to_string)
                }
            })
            .collect();
        dirs.sort();
        dirs
    }

    pub fn read(&self, rel: &str) -> Option<Vec<u8>> {
        let size = *self.files.get(rel)?;
        if size > MAX_FILE_BYTES {
            return None;
        }
        std::fs::read(self.root.join(rel)).ok()
    }

    pub fn read_str(&self, rel: &str) -> Option<String> {
        String::from_utf8(self.read(rel)?).ok()
    }
}

fn is_excluded_dir(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    if UNCONDITIONAL_EXCLUDES.contains(&name) || name.ends_with(".egg-info") {
        return true;
    }
    if CONDITIONAL_EXCLUDES.contains(&name) {
        return VENV_BUILD_MARKERS.iter().any(|m| path.join(m).exists());
    }
    false
}

/// Walk `root`, honoring .gitignore plus any `extra_ignore_files` (e.g.
/// `.fastapicloudignore` — same syntax as .gitignore, any depth, higher
/// precedence), applying built-in exclusions. Serial and byte-ordered so
/// `max_files` truncation is reproducible.
pub fn walk_fs(
    root: &Path,
    max_files: Option<u64>,
    follow_symlinks: bool,
    extra_ignore_files: &[String],
) -> FileSet {
    let mut files = BTreeMap::new();
    let mut truncated = false;

    let mut builder = ignore::WalkBuilder::new(root);
    builder
        .hidden(false) // dotfiles like .python-version matter
        .git_ignore(true)
        .git_global(false)
        .git_exclude(true)
        .parents(false)
        .require_git(false)
        .follow_links(follow_symlinks)
        .sort_by_file_name(|a, b| a.cmp(b))
        .filter_entry(|entry| {
            if entry.file_type().is_some_and(|t| t.is_dir()) {
                !is_excluded_dir(entry.path())
            } else {
                true
            }
        });
    for name in extra_ignore_files {
        builder.add_custom_ignore_filename(name);
    }
    let walker = builder.build();

    for entry in walker.flatten() {
        if !entry.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        if let Some(limit) = max_files {
            if files.len() as u64 >= limit {
                truncated = true;
                break;
            }
        }
        let Ok(rel) = entry.path().strip_prefix(root) else {
            continue;
        };
        let Some(rel_str) = rel.to_str() else {
            continue;
        };
        let rel_str = rel_str.replace(std::path::MAIN_SEPARATOR, "/");
        let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
        files.insert(rel_str, size);
    }

    FileSet {
        root: root.to_path_buf(),
        files,
        truncated,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    #[test]
    fn scan_root_does_not_inherit_parent_gitignore_rules() {
        static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);
        let suffix = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
        let parent = std::env::temp_dir().join(format!(
            "kenbun-fileset-parent-{}-{suffix}",
            std::process::id()
        ));
        let root = parent.join("fixture");
        std::fs::create_dir_all(root.join("packages/lib")).expect("create fixture tree");
        std::fs::write(parent.join(".gitignore"), "lib/\n").expect("write parent ignore");
        std::fs::write(root.join("packages/lib/package.json"), "{}\n")
            .expect("write nested manifest");

        let files = walk_fs(&root, None, false, &[]);
        assert!(files.contains("packages/lib/package.json"));
        let _ = std::fs::remove_dir_all(parent);
    }
}
