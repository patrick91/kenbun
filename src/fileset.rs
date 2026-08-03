//! Deterministic filesystem indexing and bounded file reads.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use ignore::Match;

use crate::model::FileRequest;

/// Per-file parse cap: larger files are skipped as unavailable.
pub const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;
const MAX_REQUESTS_PER_ROUND: usize = 64;
const MAX_SCRIPT_REQUESTS_PER_ROUND: usize = 16;
/// First line of a Git LFS pointer, per the LFS v1 spec.
const LFS_POINTER_PREFIX: &[u8] = b"version https://git-lfs.github.com/spec/v1";

fn is_lfs_pointer(bytes: &[u8]) -> bool {
    bytes.starts_with(LFS_POINTER_PREFIX)
}

pub fn read_bounded_path(path: &Path) -> Option<String> {
    String::from_utf8(read_bounded_bytes(path)?).ok()
}

fn read_bounded_bytes(path: &Path) -> Option<Vec<u8>> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .ok()?
        .take(MAX_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return None;
    }
    Some(bytes)
}

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
    /// Set when the `max_files` budget ran out mid-walk.
    pub truncated: bool,
    /// Filesystem entries omitted from the scan, with a stable display path
    /// and the underlying reason.
    pub issues: Vec<FileIssue>,
    /// Set when a read could not yield usable content. Tracked for both sources
    /// so local and virtual scans report completeness the same way.
    unavailable: AtomicBool,
    source: FileSource,
}

enum FileSource {
    Local,
    Virtual(VirtualSource),
}

struct VirtualSource {
    contents: BTreeMap<String, Option<Vec<u8>>>,
    max_files: Option<u64>,
    max_file_bytes: u64,
    max_depth: Option<u64>,
    script_patterns: Vec<ScriptPattern>,
    script_hints_enabled: AtomicBool,
    allowed_scripts: Mutex<BTreeSet<String>>,
    requests: Mutex<BTreeMap<String, FileRequest>>,
}

struct ScriptPattern {
    basename_only: bool,
    pattern: glob::Pattern,
}

pub struct FileIssue {
    pub path: String,
    pub message: String,
}

impl FileSet {
    #[cfg(test)]
    pub(crate) fn test_local(root: PathBuf, files: BTreeMap<String, u64>) -> Self {
        Self {
            root,
            files,
            truncated: false,
            issues: Vec::new(),
            unavailable: AtomicBool::new(false),
            source: FileSource::Local,
        }
    }

    pub fn contains(&self, rel: &str) -> bool {
        self.files.contains_key(rel)
    }

    pub fn is_virtual(&self) -> bool {
        matches!(&self.source, FileSource::Virtual(_))
    }

    pub fn max_file_bytes(&self) -> u64 {
        match &self.source {
            FileSource::Local => MAX_FILE_BYTES,
            FileSource::Virtual(source) => source.max_file_bytes,
        }
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
        let bytes = self.read_source(rel)?;
        // A pointer describes content that was never fetched. Its size is the
        // pointer's, so it slips past every cap; parsing it as the real file
        // invents facts about the repository.
        if is_lfs_pointer(&bytes) {
            self.mark_unavailable();
            return None;
        }
        Some(bytes)
    }

    fn read_source(&self, rel: &str) -> Option<Vec<u8>> {
        let size = *self.files.get(rel)?;
        let max_file_bytes = match &self.source {
            FileSource::Local => MAX_FILE_BYTES,
            FileSource::Virtual(source) => source.max_file_bytes,
        };
        if size > max_file_bytes {
            self.mark_unavailable();
            return None;
        }
        match &self.source {
            FileSource::Local => {
                let bytes = read_bounded_bytes(&self.root.join(rel));
                if bytes.is_none() {
                    self.mark_unavailable();
                }
                bytes
            }
            FileSource::Virtual(source) => match source.contents.get(rel) {
                Some(Some(bytes)) if bytes.len() as u64 <= source.max_file_bytes => {
                    Some(bytes.clone())
                }
                Some(_) => {
                    self.mark_unavailable();
                    None
                }
                None => {
                    let explicitly_allowed = source
                        .allowed_scripts
                        .lock()
                        .expect("lock poisoned")
                        .contains(rel);
                    let hint_allowed = source.script_hints_enabled.load(Ordering::Relaxed)
                        && source
                            .script_patterns
                            .iter()
                            .any(|pattern| pattern.matches(rel));
                    if is_script(rel)
                        && !is_manifest_or_config_script(rel)
                        && !explicitly_allowed
                        && !hint_allowed
                    {
                        return None;
                    }
                    source
                        .requests
                        .lock()
                        .expect("lock poisoned")
                        .entry(rel.to_string())
                        .or_insert_with(|| {
                            let (reason, priority) = request_kind(rel);
                            FileRequest {
                                path: rel.to_string(),
                                reason: reason.to_string(),
                                priority,
                            }
                        });
                    None
                }
            },
        }
    }

    pub fn read_str(&self, rel: &str) -> Option<String> {
        match String::from_utf8(self.read(rel)?) {
            Ok(source) => Some(source),
            Err(_) => {
                self.mark_unavailable();
                None
            }
        }
    }

    pub fn allow_script(&self, rel: String) {
        if let FileSource::Virtual(source) = &self.source {
            source
                .allowed_scripts
                .lock()
                .expect("lock poisoned")
                .insert(rel);
        }
    }

    pub fn enable_script_hints(&self) {
        if let FileSource::Virtual(source) = &self.source {
            source.script_hints_enabled.store(true, Ordering::Relaxed);
        }
    }

    pub fn hinted_scripts(&self, dir: &str) -> Vec<String> {
        let FileSource::Virtual(source) = &self.source else {
            return Vec::new();
        };
        if !source.script_hints_enabled.load(Ordering::Relaxed) {
            return Vec::new();
        }
        let prefix = if dir.is_empty() || dir == "." {
            String::new()
        } else {
            format!("{dir}/")
        };
        let mut seen = BTreeSet::new();
        let mut scripts = Vec::new();
        for pattern in &source.script_patterns {
            for path in self.files.keys() {
                if !path.starts_with(&prefix)
                    || !is_script(path)
                    || !pattern.matches(path)
                    || !seen.insert(path.clone())
                {
                    continue;
                }
                scripts.push(path.clone());
            }
        }
        scripts
    }

    pub fn is_pending(&self, rel: &str) -> bool {
        matches!(&self.source, FileSource::Virtual(source) if source.requests.lock().expect("lock poisoned").contains_key(rel))
    }

    pub fn unavailable_seen(&self) -> bool {
        self.unavailable.load(Ordering::Relaxed)
            || matches!(&self.source, FileSource::Local) && !self.issues.is_empty()
    }

    pub fn requests(&self) -> Vec<FileRequest> {
        let FileSource::Virtual(source) = &self.source else {
            return Vec::new();
        };
        let requests = source.requests.lock().expect("lock poisoned");
        let Some(priority) = requests.values().map(|request| request.priority).min() else {
            return Vec::new();
        };
        let round_limit = if priority >= 40 {
            MAX_SCRIPT_REQUESTS_PER_ROUND
        } else {
            MAX_REQUESTS_PER_ROUND
        };
        let provided_files = source
            .contents
            .keys()
            .filter(|path| !exceeds_max_depth(path, source.max_depth))
            .count() as u64;
        let remaining_files = source
            .max_files
            .map(|limit| limit.saturating_sub(provided_files))
            .unwrap_or(u64::MAX);
        if remaining_files == 0 {
            self.mark_unavailable();
            return Vec::new();
        }
        let request_limit = round_limit.min(usize::try_from(remaining_files).unwrap_or(usize::MAX));
        requests
            .values()
            .filter(|request| request.priority == priority)
            .take(request_limit)
            .cloned()
            .collect()
    }

    pub fn has_ignore_requests(&self) -> bool {
        matches!(&self.source, FileSource::Virtual(source) if source.requests.lock().expect("lock poisoned").values().any(|request| request.priority == 0))
    }

    fn mark_unavailable(&self) {
        self.unavailable.store(true, Ordering::Relaxed);
    }
}

impl ScriptPattern {
    fn matches(&self, rel: &str) -> bool {
        let candidate = if self.basename_only {
            rel.rsplit('/').next().unwrap_or(rel)
        } else {
            rel
        };
        self.pattern.matches_with(
            candidate,
            glob::MatchOptions {
                require_literal_separator: true,
                ..glob::MatchOptions::default()
            },
        )
    }
}

fn is_script(path: &str) -> bool {
    matches!(
        path.rsplit('.').next(),
        Some("py" | "pyw" | "js" | "jsx" | "ts" | "tsx" | "mjs" | "cjs" | "mts" | "cts")
    )
}

fn is_manifest_or_config_script(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    matches!(name, "setup.py" | "manage.py") || name.contains(".config.")
}

fn request_kind(path: &str) -> (&'static str, u32) {
    let name = path.rsplit('/').next().unwrap_or(path);
    if name == ".gitignore" {
        ("ignore rules", 0)
    } else if name == "pyproject.toml"
        || name == "package.json"
        || name == "Pipfile"
        || name == "setup.py"
        || name == "setup.cfg"
        || name == "pnpm-workspace.yaml"
        || (name.starts_with("requirements") && name.ends_with(".txt"))
        || (path.contains("/requirements/") && name.ends_with(".txt"))
    {
        ("application manifest", 10)
    } else if matches!(
        name,
        ".python-version" | ".node-version" | ".nvmrc" | ".tool-versions"
    ) {
        ("runtime metadata", 20)
    } else if is_script(path) {
        ("script discovery hint", 40)
    } else {
        ("application configuration", 30)
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

/// Directories above a file: `main.py` is 0, `app/main.py` is 1. Applications
/// live near the top of a repository, so paths below the limit cannot hold one.
/// This is an exclusion in the same family as `node_modules` and `.venv`, not a
/// budget running out, so it does not make a result partial: saying otherwise
/// would mark ordinary repositories incomplete for files that never mattered.
fn exceeds_max_depth(path: &str, max_depth: Option<u64>) -> bool {
    max_depth.is_some_and(|limit| path.matches('/').count() as u64 > limit)
}

pub fn virtual_files(
    entries: Vec<(String, u64, bool)>,
    mut contents: BTreeMap<String, Option<Vec<u8>>>,
    script_patterns: Vec<String>,
    max_files: Option<u64>,
    max_file_bytes: u64,
    max_depth: Option<u64>,
) -> Result<FileSet, String> {
    let mut inventory_symlinks = BTreeMap::new();
    let mut all_entries = BTreeMap::new();
    for (path, size, is_symlink) in entries {
        validate_relative_path(&path)?;
        if inventory_symlinks
            .insert(path.clone(), is_symlink)
            .is_some()
        {
            return Err(format!("duplicate file inventory path: {path}"));
        }
        if !is_symlink {
            all_entries.insert(path, size);
        }
    }
    for path in contents.keys() {
        validate_relative_path(path)?;
        if !inventory_symlinks.contains_key(path) {
            return Err(format!(
                "content path is not present in the inventory: {path}"
            ));
        }
    }
    contents.retain(|path, _| !inventory_symlinks[path]);

    let mut compiled_patterns = Vec::new();
    let mut seen_patterns = BTreeSet::new();
    for raw in script_patterns {
        if raw.is_empty()
            || raw.starts_with('/')
            || raw.contains('\\')
            || raw.split('/').any(|part| matches!(part, "." | ".."))
        {
            return Err(format!("invalid script pattern: {raw}"));
        }
        if !seen_patterns.insert(raw.clone()) {
            continue;
        }
        let pattern = glob::Pattern::new(&raw)
            .map_err(|error| format!("invalid script pattern {raw:?}: {error}"))?;
        compiled_patterns.push(ScriptPattern {
            basename_only: !raw.contains('/'),
            pattern,
        });
    }

    let mut issues = Vec::new();
    let mut requests = BTreeMap::new();
    let mut ignore_matchers = Vec::new();
    let mut unavailable_seen = false;
    for path in all_entries
        .keys()
        .filter(|path| path.rsplit('/').next() == Some(".gitignore"))
        .filter(|path| !exceeds_max_depth(path, max_depth))
    {
        if all_entries[path] > max_file_bytes {
            unavailable_seen = true;
            continue;
        }
        match contents.get(path) {
            None => {
                requests.insert(
                    path.clone(),
                    FileRequest {
                        path: path.clone(),
                        reason: "ignore rules".to_string(),
                        priority: 0,
                    },
                );
            }
            Some(None) => unavailable_seen = true,
            Some(Some(bytes)) if bytes.len() as u64 > max_file_bytes => {
                unavailable_seen = true;
            }
            Some(Some(bytes)) if is_lfs_pointer(bytes) => {
                unavailable_seen = true;
                issues.push(FileIssue {
                    path: path.clone(),
                    message: "Git LFS pointer does not contain the file content".to_string(),
                });
            }
            Some(Some(bytes)) => match std::str::from_utf8(bytes) {
                Ok(source) => {
                    let dir = path.rsplit_once('/').map(|(dir, _)| dir).unwrap_or("");
                    let root = if dir.is_empty() { "." } else { dir };
                    let mut builder = ignore::gitignore::GitignoreBuilder::new(root);
                    let mut valid = true;
                    for (line_number, line) in source.lines().enumerate() {
                        if let Err(error) = builder.add_line(Some(PathBuf::from(path)), line) {
                            valid = false;
                            issues.push(FileIssue {
                                path: path.clone(),
                                message: format!(
                                    "invalid ignore rule on line {}: {error}",
                                    line_number + 1
                                ),
                            });
                        }
                    }
                    match builder.build() {
                        Ok(matcher) => ignore_matchers.push((dir.to_string(), matcher)),
                        Err(error) => {
                            valid = false;
                            issues.push(FileIssue {
                                path: path.clone(),
                                message: format!("invalid ignore rules: {error}"),
                            });
                        }
                    }
                    unavailable_seen |= !valid;
                }
                Err(_) => {
                    unavailable_seen = true;
                    issues.push(FileIssue {
                        path: path.clone(),
                        message: "ignore file is not valid UTF-8".to_string(),
                    });
                }
            },
        }
    }
    ignore_matchers.sort_by(|a, b| {
        a.0.matches('/')
            .count()
            .cmp(&b.0.matches('/').count())
            .then(a.0.cmp(&b.0))
    });

    let all_paths: BTreeSet<String> = all_entries.keys().cloned().collect();
    let files = all_entries
        .into_iter()
        .filter(|(path, _)| {
            !is_builtin_excluded(path, &all_paths)
                && !is_ignored(path, &ignore_matchers)
                && !exceeds_max_depth(path, max_depth)
        })
        .collect();

    Ok(FileSet {
        root: PathBuf::from("."),
        files,
        truncated: false,
        issues,
        unavailable: AtomicBool::new(unavailable_seen),
        source: FileSource::Virtual(VirtualSource {
            contents,
            max_files,
            max_file_bytes,
            max_depth,
            script_patterns: compiled_patterns,
            script_hints_enabled: AtomicBool::new(false),
            allowed_scripts: Mutex::new(BTreeSet::new()),
            requests: Mutex::new(requests),
        }),
    })
}

fn validate_relative_path(path: &str) -> Result<(), String> {
    if path.is_empty()
        || path.starts_with('/')
        || path.contains('\\')
        || path
            .split('/')
            .any(|part| part.is_empty() || matches!(part, "." | ".."))
    {
        return Err(format!(
            "file paths must be normalized repository-relative POSIX paths: {path:?}"
        ));
    }
    Ok(())
}

fn is_builtin_excluded(path: &str, all_paths: &BTreeSet<String>) -> bool {
    let parts: Vec<&str> = path.split('/').collect();
    for index in 0..parts.len().saturating_sub(1) {
        let name = parts[index];
        if UNCONDITIONAL_EXCLUDES.contains(&name) || name.ends_with(".egg-info") {
            return true;
        }
        if CONDITIONAL_EXCLUDES.contains(&name) {
            let dir = parts[..=index].join("/");
            if VENV_BUILD_MARKERS
                .iter()
                .any(|marker| all_paths.contains(&format!("{dir}/{marker}")))
            {
                return true;
            }
        }
    }
    false
}

fn is_ignored(path: &str, matchers: &[(String, ignore::gitignore::Gitignore)]) -> bool {
    let mut ignored = false;
    for (dir, matcher) in matchers {
        if !dir.is_empty() && path != dir && !path.starts_with(&format!("{dir}/")) {
            continue;
        }
        match matcher.matched_path_or_any_parents(path, false) {
            Match::Ignore(_) => ignored = true,
            Match::Whitelist(_) => ignored = false,
            Match::None => {}
        }
    }
    ignored
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
    max_depth: Option<u64>,
) -> FileSet {
    let mut files = BTreeMap::new();
    let mut truncated = false;
    let mut issues = Vec::new();
    let mut unavailable_seen = false;

    if !root.is_dir() {
        issues.push(FileIssue {
            path: root.to_string_lossy().into_owned(),
            message: "scan root does not exist or is not a directory".to_string(),
        });
        return FileSet {
            root: root.to_path_buf(),
            files,
            truncated,
            issues,
            unavailable: AtomicBool::new(false),
            source: FileSource::Local,
        };
    }
    let canonical_root = std::fs::canonicalize(root).ok();

    let mut builder = ignore::WalkBuilder::new(root);
    let containment_root = canonical_root.clone();
    let walk_root = root.to_path_buf();
    builder
        .hidden(false) // dotfiles like .python-version matter
        .ignore(false) // `.ignore` is not part of the documented upload set
        .git_ignore(true)
        .git_global(false)
        .git_exclude(true)
        .parents(false)
        .require_git(false)
        .follow_links(follow_symlinks)
        .sort_by_file_name(|a, b| a.cmp(b))
        .filter_entry(move |entry| {
            if follow_symlinks
                && containment_root.as_ref().is_some_and(|canonical_root| {
                    std::fs::canonicalize(entry.path())
                        .is_ok_and(|canonical_path| !canonical_path.starts_with(canonical_root))
                })
            {
                return false;
            }
            if !entry.file_type().is_some_and(|t| t.is_dir()) {
                return true;
            }
            if is_excluded_dir(entry.path()) {
                return false;
            }
            !entry.path().strip_prefix(&walk_root).is_ok_and(|relative| {
                max_depth.is_some_and(|limit| relative.components().count() as u64 > limit)
            })
        });
    for name in extra_ignore_files {
        builder.add_custom_ignore_filename(name);
    }
    let walker = builder.build();

    for result in walker {
        let entry = match result {
            Ok(entry) => entry,
            Err(error) => {
                issues.push(FileIssue {
                    path: root.to_string_lossy().into_owned(),
                    message: error.to_string(),
                });
                continue;
            }
        };
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
            issues.push(FileIssue {
                path: rel.to_string_lossy().into_owned(),
                message: "path is not valid UTF-8".to_string(),
            });
            continue;
        };
        let rel_str = rel_str.replace(std::path::MAIN_SEPARATOR, "/");
        if exceeds_max_depth(&rel_str, max_depth) {
            continue;
        }
        if follow_symlinks
            && canonical_root.as_ref().is_some_and(|canonical_root| {
                std::fs::canonicalize(entry.path())
                    .is_ok_and(|canonical_path| !canonical_path.starts_with(canonical_root))
            })
        {
            issues.push(FileIssue {
                path: rel_str,
                message: "symlink target escapes the scan root".to_string(),
            });
            continue;
        }
        let size = match entry.metadata() {
            Ok(metadata) => metadata.len(),
            Err(error) => {
                issues.push(FileIssue {
                    path: rel_str,
                    message: format!("metadata unavailable: {error}"),
                });
                continue;
            }
        };
        if rel_str.rsplit('/').next() == Some(".gitignore")
            && read_bounded_bytes(entry.path()).is_some_and(|bytes| is_lfs_pointer(&bytes))
        {
            unavailable_seen = true;
            issues.push(FileIssue {
                path: rel_str.clone(),
                message: "Git LFS pointer does not contain the file content".to_string(),
            });
        }
        files.insert(rel_str, size);
    }
    FileSet {
        root: root.to_path_buf(),
        files,
        truncated,
        issues,
        unavailable: AtomicBool::new(unavailable_seen),
        source: FileSource::Local,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn depth_limited(paths: &[&str], max_depth: Option<u64>) -> FileSet {
        virtual_files(
            paths
                .iter()
                .map(|path| ((*path).to_string(), 10, false))
                .collect(),
            BTreeMap::new(),
            Vec::new(),
            None,
            1024,
            max_depth,
        )
        .expect("inventory is valid")
    }

    #[test]
    fn max_depth_counts_directories_above_the_file() {
        let paths = ["main.py", "app/main.py", "a/b/main.py", "a/b/c/main.py"];

        let unlimited = depth_limited(&paths, None);
        assert_eq!(unlimited.files.len(), 4);
        assert!(!unlimited.truncated);

        let flat = depth_limited(&paths, Some(0));
        assert_eq!(flat.files.keys().collect::<Vec<_>>(), vec!["main.py"]);

        let nested = depth_limited(&paths, Some(2));
        assert_eq!(
            nested.files.keys().collect::<Vec<_>>(),
            vec!["a/b/main.py", "app/main.py", "main.py"]
        );
    }

    #[test]
    fn depth_pruning_does_not_make_a_scan_partial() {
        // Depth is a statement about where applications live, like the
        // `node_modules` exclusion, not a budget running out. Reporting it as
        // truncation would mark ordinary repositories incomplete over files
        // that could never have held an application.
        let pruned = depth_limited(&["a/b/main.py"], Some(0));
        assert!(pruned.files.is_empty());
        assert!(!pruned.truncated);
    }

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

        let files = walk_fs(&root, None, false, &[], None);
        assert!(files.contains("packages/lib/package.json"));
        let _ = std::fs::remove_dir_all(parent);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn non_utf8_file_names_are_reported() {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt;

        let root =
            std::env::temp_dir().join(format!("kenbun-fileset-non-utf8-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("create fixture");
        let invalid = OsString::from_vec(vec![b'f', b'o', 0x80]);
        std::fs::write(root.join(invalid), b"data").expect("write non-UTF-8 fixture");

        let files = walk_fs(&root, None, false, &[], None);
        assert!(files.files.is_empty());
        assert!(files
            .issues
            .iter()
            .any(|issue| issue.message.contains("not valid UTF-8")));
        let _ = std::fs::remove_dir_all(root);
    }
}
