//! Deterministic filesystem indexing and bounded file reads.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use ignore::Match;

use crate::model::{FileEntry, WantFile};

/// Per-file parse cap: larger files are skipped as unavailable.
pub const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;
const MAX_WANTS_PER_ROUND: usize = 64;
const MAX_SCRIPT_WANTS_PER_ROUND: usize = 16;

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
    pub truncated: bool,
    /// Filesystem entries omitted from the scan, with a stable display path
    /// and the underlying reason.
    pub issues: Vec<FileIssue>,
    source: FileSource,
}

enum FileSource {
    Local,
    Virtual(VirtualSource),
}

struct VirtualSource {
    contents: BTreeMap<String, Option<Vec<u8>>>,
    blob_shas: BTreeMap<String, String>,
    script_patterns: Vec<ScriptPattern>,
    script_hints_enabled: AtomicBool,
    allowed_scripts: Mutex<BTreeSet<String>>,
    wants: Mutex<BTreeMap<String, WantFile>>,
    unavailable_seen: AtomicBool,
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
            source: FileSource::Local,
        }
    }

    pub fn contains(&self, rel: &str) -> bool {
        self.files.contains_key(rel)
    }

    pub fn is_virtual(&self) -> bool {
        matches!(&self.source, FileSource::Virtual(_))
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
                Some(Some(bytes)) if bytes.len() as u64 <= MAX_FILE_BYTES => Some(bytes.clone()),
                Some(_) => {
                    source.unavailable_seen.store(true, Ordering::Relaxed);
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
                        .wants
                        .lock()
                        .expect("lock poisoned")
                        .entry(rel.to_string())
                        .or_insert_with(|| {
                            let (reason, priority) = request_kind(rel);
                            WantFile {
                                path: rel.to_string(),
                                reason: reason.to_string(),
                                priority,
                                max_bytes: MAX_FILE_BYTES,
                                blob_sha: source.blob_shas.get(rel).cloned(),
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
        matches!(&self.source, FileSource::Virtual(source) if source.wants.lock().expect("lock poisoned").contains_key(rel))
    }

    pub fn unavailable_seen(&self) -> bool {
        matches!(&self.source, FileSource::Virtual(source) if source.unavailable_seen.load(Ordering::Relaxed))
            || matches!(&self.source, FileSource::Local) && !self.issues.is_empty()
    }

    pub fn wants(&self) -> Vec<WantFile> {
        let FileSource::Virtual(source) = &self.source else {
            return Vec::new();
        };
        let wants = source.wants.lock().expect("lock poisoned");
        let Some(priority) = wants.values().map(|want| want.priority).min() else {
            return Vec::new();
        };
        let limit = if priority >= 40 {
            MAX_SCRIPT_WANTS_PER_ROUND
        } else {
            MAX_WANTS_PER_ROUND
        };
        wants
            .values()
            .filter(|want| want.priority == priority)
            .take(limit)
            .cloned()
            .collect()
    }

    pub fn has_ignore_wants(&self) -> bool {
        matches!(&self.source, FileSource::Virtual(source) if source.wants.lock().expect("lock poisoned").values().any(|want| want.priority == 0))
    }

    fn mark_unavailable(&self) {
        if let FileSource::Virtual(source) = &self.source {
            source.unavailable_seen.store(true, Ordering::Relaxed);
        }
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
        "uv.lock"
            | "pylock.toml"
            | "poetry.lock"
            | "pdm.lock"
            | "Pipfile.lock"
            | "package-lock.json"
            | "npm-shrinkwrap.json"
            | "pnpm-lock.yaml"
            | "yarn.lock"
            | "bun.lock"
            | "bun.lockb"
            | ".python-version"
            | ".node-version"
            | ".nvmrc"
            | ".tool-versions"
    ) {
        ("runtime or lock metadata", 20)
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

pub fn virtual_files(
    entries: Vec<FileEntry>,
    contents: BTreeMap<String, Option<Vec<u8>>>,
    script_patterns: Vec<String>,
) -> Result<FileSet, String> {
    let mut all_entries = BTreeMap::new();
    let mut blob_shas = BTreeMap::new();
    for entry in entries {
        validate_relative_path(&entry.path)?;
        if all_entries.insert(entry.path.clone(), entry.size).is_some() {
            return Err(format!("duplicate file inventory path: {}", entry.path));
        }
        if let Some(blob_sha) = entry.blob_sha {
            blob_shas.insert(entry.path, blob_sha);
        }
    }
    for path in contents.keys() {
        validate_relative_path(path)?;
        if !all_entries.contains_key(path) {
            return Err(format!(
                "content path is not present in the inventory: {path}"
            ));
        }
    }

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
    let mut wants = BTreeMap::new();
    let mut ignore_matchers = Vec::new();
    let mut unavailable_seen = false;
    for path in all_entries
        .keys()
        .filter(|path| path.rsplit('/').next() == Some(".gitignore"))
    {
        match contents.get(path) {
            None => {
                wants.insert(
                    path.clone(),
                    WantFile {
                        path: path.clone(),
                        reason: "ignore rules".to_string(),
                        priority: 0,
                        max_bytes: MAX_FILE_BYTES,
                        blob_sha: blob_shas.get(path).cloned(),
                    },
                );
            }
            Some(None) => unavailable_seen = true,
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
            !is_builtin_excluded(path, &all_paths) && !is_ignored(path, &ignore_matchers)
        })
        .collect();

    Ok(FileSet {
        root: PathBuf::from("."),
        files,
        truncated: false,
        issues,
        source: FileSource::Virtual(VirtualSource {
            contents,
            blob_shas,
            script_patterns: compiled_patterns,
            script_hints_enabled: AtomicBool::new(false),
            allowed_scripts: Mutex::new(BTreeSet::new()),
            wants: Mutex::new(wants),
            unavailable_seen: AtomicBool::new(unavailable_seen),
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
) -> FileSet {
    let mut files = BTreeMap::new();
    let mut truncated = false;
    let mut issues = Vec::new();

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
            source: FileSource::Local,
        };
    }
    let canonical_root = std::fs::canonicalize(root).ok();

    let mut builder = ignore::WalkBuilder::new(root);
    let containment_root = canonical_root.clone();
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
        files.insert(rel_str, size);
    }

    FileSet {
        root: root.to_path_buf(),
        files,
        truncated,
        issues,
        source: FileSource::Local,
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

        let files = walk_fs(&root, None, false, &[]);
        assert!(files.files.is_empty());
        assert!(files
            .issues
            .iter()
            .any(|issue| issue.message.contains("not valid UTF-8")));
        let _ = std::fs::remove_dir_all(root);
    }
}
