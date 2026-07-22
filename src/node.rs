//! Raw Node/JavaScript project discovery.
//!
//! This module deliberately stops short of constructing public scan-model
//! objects.  It collects deterministic, non-executing facts that the root
//! scanner can later reconcile with Python (and other ecosystem) projects.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Map, Value};

use crate::fileset::FileSet;
use crate::runtime::{self, RuntimePin};

const LOCKFILE_NAMES: &[(&str, &str)] = &[
    ("package-lock.json", "npm"),
    ("npm-shrinkwrap.json", "npm"),
    ("pnpm-lock.yaml", "pnpm"),
    ("yarn.lock", "yarn"),
    ("bun.lock", "bun"),
    ("bun.lockb", "bun"),
];

const CONFIG_EXTENSIONS: &[&str] = &["js", "mjs", "cjs", "ts", "mts", "cts"];
const CONFIG_PREFIXES: &[&str] = &[
    "astro",
    "next",
    "nuxt",
    "svelte",
    "vite",
    "react-router",
    "remix",
];

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct RawNodeDiscovery {
    pub packages: Vec<RawNodePackage>,
    pub workspaces: Vec<RawNodeWorkspace>,
    pub parse_errors: Vec<RawNodeParseError>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct RawNodePackage {
    /// Package directory relative to the FileSet root (`.` for the root).
    pub path: String,
    pub manifest_path: String,
    /// True when package.json was valid JSON with an object at the root.
    pub parsed: bool,
    pub name: Option<String>,
    pub dependencies: BTreeMap<String, String>,
    pub dev_dependencies: BTreeMap<String, String>,
    pub optional_dependencies: BTreeMap<String, String>,
    pub scripts: BTreeMap<String, String>,
    /// The unmodified packageManager value, if it was a string.
    pub explicit_package_manager: Option<String>,
    pub requires_node: Option<String>,
    pub version_pins: Vec<RuntimePin>,
    pub package_manager: Option<RawPackageManager>,
    /// Candidates at the nearest lock/workspace evidence level. More than one
    /// means the evidence was ambiguous and `package_manager` is None.
    pub package_manager_candidates: Vec<String>,
    pub package_manager_evidence: Vec<String>,
    pub declares_workspace: bool,
    pub workspace_patterns: Vec<String>,
    /// Same-directory lockfile paths.
    pub lockfiles: Vec<String>,
    /// Same-directory framework/build/language config paths.
    pub config_files: Vec<String>,
    pub index_html: Option<String>,
    pub language: RawLanguageSignals,
    pub technologies: Vec<RawTechnologySignal>,
    pub vite: RawViteSignals,
    pub inertia: RawInertiaSignals,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct RawNodeWorkspace {
    pub path: String,
    /// package.json and/or pnpm-workspace.yaml paths that declared it.
    pub sources: Vec<String>,
    pub patterns: Vec<String>,
    /// Pattern-expanded package directories. The root is only present when a
    /// pattern explicitly matches it; callers can inspect `has_root_package`.
    pub members: Vec<String>,
    pub unmatched_patterns: Vec<String>,
    pub has_root_package: bool,
    pub package_manager: Option<RawPackageManager>,
    pub package_manager_candidates: Vec<String>,
    pub package_manager_evidence: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RawPackageManager {
    pub name: String,
    pub source: String,
    pub explicit: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct RawLanguageSignals {
    pub typescript: bool,
    pub javascript: bool,
    /// TypeScript wins when both occur, because JS config files are common in
    /// otherwise-TypeScript projects. The individual booleans retain the raw
    /// mixed-language fact.
    pub primary: Option<String>,
    pub evidence: Vec<String>,
    pub typescript_evidence: Vec<String>,
    pub javascript_evidence: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RawTechnologySignal {
    pub id: String,
    pub kind: String,
    pub evidence: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct RawViteSignals {
    pub direct_dependency: bool,
    pub config_files: Vec<String>,
    pub script_names: Vec<String>,
    pub build_script_invokes_vite_build: bool,
    pub has_index_html: bool,
    /// Deliberately strict: a direct vite dependency, an explicit `build`
    /// script invocation of `vite build`, and same-root index.html.
    pub standalone: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct RawInertiaSignals {
    pub adapters: Vec<String>,
    pub packages: Vec<String>,
    pub vite_helper: bool,
    pub corroborated: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RawNodeParseError {
    pub path: String,
    pub message: String,
}

#[derive(Clone, Debug, Default)]
struct PackageManifest {
    parsed: bool,
    name: Option<String>,
    dependencies: BTreeMap<String, String>,
    dev_dependencies: BTreeMap<String, String>,
    optional_dependencies: BTreeMap<String, String>,
    scripts: BTreeMap<String, String>,
    package_manager: Option<String>,
    requires_node: Option<String>,
    declares_workspace: bool,
    workspace_patterns: Vec<String>,
}

#[derive(Default)]
struct WorkspaceBuilder {
    sources: BTreeSet<String>,
    patterns: BTreeSet<String>,
}

/// Discover all package.json projects and JavaScript workspaces in a FileSet.
///
/// The function never executes repository code and never assumes npm merely
/// because package.json exists. Malformed inputs are retained as package or
/// workspace locations and reported through `parse_errors`.
pub(crate) fn discover(fs: &FileSet) -> RawNodeDiscovery {
    let mut parse_errors = Vec::new();
    let mut manifests: BTreeMap<String, PackageManifest> = BTreeMap::new();

    for dir in fs.dirs_with("package.json") {
        let path = join(&dir, "package.json");
        let manifest = match fs.read_str(&path) {
            Some(source) => parse_package_json(&path, &source, &mut parse_errors),
            None => {
                if !fs.is_pending(&path) {
                    parse_errors.push(raw_error(
                        &path,
                        "package.json is unavailable, too large, non-UTF-8, or unreadable",
                    ));
                }
                PackageManifest::default()
            }
        };
        manifests.insert(dir, manifest);
    }

    let package_dirs: Vec<String> = manifests.keys().cloned().collect();
    let mut source_boundaries: BTreeSet<String> = package_dirs.iter().cloned().collect();
    for marker in [
        "pyproject.toml",
        "requirements.txt",
        "Pipfile",
        "setup.py",
        "manage.py",
    ] {
        source_boundaries.extend(fs.dirs_with(marker));
    }
    let owned_source_files = source_files_by_owner(fs, &source_boundaries);
    let mut workspace_builders: BTreeMap<String, WorkspaceBuilder> = BTreeMap::new();

    for (dir, manifest) in &manifests {
        if manifest.declares_workspace {
            let builder = workspace_builders.entry(dir.clone()).or_default();
            builder.sources.insert(join(dir, "package.json"));
            builder
                .patterns
                .extend(manifest.workspace_patterns.iter().cloned());
        }
    }

    for dir in fs.dirs_with("pnpm-workspace.yaml") {
        let path = join(&dir, "pnpm-workspace.yaml");
        let builder = workspace_builders.entry(dir).or_default();
        builder.sources.insert(path.clone());
        match fs.read_str(&path) {
            Some(source) => {
                let (patterns, messages) = parse_pnpm_workspace_yaml(&source);
                builder.patterns.extend(patterns);
                parse_errors.extend(
                    messages
                        .into_iter()
                        .map(|message| raw_error(&path, message)),
                );
            }
            None if !fs.is_pending(&path) => parse_errors.push(raw_error(
                &path,
                "pnpm-workspace.yaml is unavailable, too large, non-UTF-8, or unreadable",
            )),
            None => {}
        }
    }

    let mut packages = Vec::new();
    for (dir, manifest) in &manifests {
        let lockfiles = same_root_lockfiles(fs, dir);
        let config_files = same_root_config_files(fs, dir);
        let index_html_path = join(dir, "index.html");
        let index_html = fs.contains(&index_html_path).then_some(index_html_path);
        let (package_manager, manager_candidates, manager_evidence) =
            infer_package_manager(fs, dir, &manifests);
        let version_pins = runtime::node_version_pins(fs, dir);
        let language = classify_language(
            fs,
            dir,
            owned_source_files
                .get(dir)
                .map(Vec::as_slice)
                .unwrap_or(&[]),
            manifest,
        );
        let (technologies, vite, inertia) = classify_technologies(
            fs,
            dir,
            manifest,
            &config_files,
            index_html.is_some(),
            &language,
        );

        packages.push(RawNodePackage {
            path: display_dir(dir),
            manifest_path: join(dir, "package.json"),
            parsed: manifest.parsed,
            name: manifest.name.clone(),
            dependencies: manifest.dependencies.clone(),
            dev_dependencies: manifest.dev_dependencies.clone(),
            optional_dependencies: manifest.optional_dependencies.clone(),
            scripts: manifest.scripts.clone(),
            explicit_package_manager: manifest.package_manager.clone(),
            requires_node: manifest.requires_node.clone(),
            version_pins,
            package_manager,
            package_manager_candidates: manager_candidates,
            package_manager_evidence: manager_evidence,
            declares_workspace: manifest.declares_workspace,
            workspace_patterns: manifest.workspace_patterns.clone(),
            lockfiles,
            config_files,
            index_html,
            language,
            technologies,
            vite,
            inertia,
        });
    }

    let mut workspaces = Vec::new();
    for (dir, builder) in workspace_builders {
        let patterns: Vec<String> = builder.patterns.into_iter().collect();
        let sources: Vec<String> = builder.sources.into_iter().collect();
        let (members, unmatched, glob_errors) = expand_workspace_patterns(&patterns, &package_dirs);
        let error_path = sources
            .first()
            .cloned()
            .unwrap_or_else(|| display_dir(&dir));
        parse_errors.extend(
            glob_errors
                .into_iter()
                .map(|message| raw_error(&error_path, message)),
        );
        let (package_manager, candidates, evidence) = infer_package_manager(fs, &dir, &manifests);
        workspaces.push(RawNodeWorkspace {
            path: display_dir(&dir),
            sources,
            patterns,
            members,
            unmatched_patterns: unmatched,
            has_root_package: manifests.contains_key(&dir),
            package_manager,
            package_manager_candidates: candidates,
            package_manager_evidence: evidence,
        });
    }

    packages.sort_by(|a, b| a.path.cmp(&b.path));
    workspaces.sort_by(|a, b| a.path.cmp(&b.path));
    parse_errors.sort_by(|a, b| (&a.path, &a.message).cmp(&(&b.path, &b.message)));
    parse_errors.dedup();

    RawNodeDiscovery {
        packages,
        workspaces,
        parse_errors,
    }
}

fn parse_package_json(
    path: &str,
    source: &str,
    errors: &mut Vec<RawNodeParseError>,
) -> PackageManifest {
    let value: Value = match serde_json::from_str(source) {
        Ok(value) => value,
        Err(error) => {
            errors.push(raw_error(path, format!("invalid JSON: {error}")));
            return PackageManifest::default();
        }
    };
    let Some(object) = value.as_object() else {
        errors.push(raw_error(path, "package.json root must be an object"));
        return PackageManifest::default();
    };

    let name = optional_string(object, "name", path, errors);
    let package_manager = optional_string(object, "packageManager", path, errors);
    let requires_node = string_map(object, "engines", path, errors).remove("node");
    let dependencies = string_map(object, "dependencies", path, errors);
    let dev_dependencies = string_map(object, "devDependencies", path, errors);
    let optional_dependencies = string_map(object, "optionalDependencies", path, errors);
    let scripts = string_map(object, "scripts", path, errors);
    let (declares_workspace, mut workspace_patterns) =
        package_workspace_patterns(object.get("workspaces"), path, errors);
    workspace_patterns.sort();
    workspace_patterns.dedup();

    PackageManifest {
        parsed: true,
        name,
        dependencies,
        dev_dependencies,
        optional_dependencies,
        scripts,
        package_manager,
        requires_node,
        declares_workspace,
        workspace_patterns,
    }
}

fn optional_string(
    object: &Map<String, Value>,
    key: &str,
    path: &str,
    errors: &mut Vec<RawNodeParseError>,
) -> Option<String> {
    match object.get(key) {
        None | Some(Value::Null) => None,
        Some(Value::String(value)) => Some(value.clone()),
        Some(_) => {
            errors.push(raw_error(path, format!("`{key}` must be a string")));
            None
        }
    }
}

fn string_map(
    object: &Map<String, Value>,
    key: &str,
    path: &str,
    errors: &mut Vec<RawNodeParseError>,
) -> BTreeMap<String, String> {
    let Some(value) = object.get(key) else {
        return BTreeMap::new();
    };
    let Some(values) = value.as_object() else {
        errors.push(raw_error(path, format!("`{key}` must be an object")));
        return BTreeMap::new();
    };
    let mut out = BTreeMap::new();
    for (name, value) in values {
        if let Some(value) = value.as_str() {
            out.insert(name.clone(), value.to_string());
        } else {
            errors.push(raw_error(path, format!("`{key}.{name}` must be a string")));
        }
    }
    out
}

fn package_workspace_patterns(
    value: Option<&Value>,
    path: &str,
    errors: &mut Vec<RawNodeParseError>,
) -> (bool, Vec<String>) {
    let Some(value) = value else {
        return (false, Vec::new());
    };
    let packages = match value {
        Value::Array(packages) => packages,
        Value::Object(object) => {
            let Some(packages) = object.get("packages") else {
                errors.push(raw_error(
                    path,
                    "`workspaces` object must contain a `packages` array",
                ));
                return (true, Vec::new());
            };
            let Some(packages) = packages.as_array() else {
                errors.push(raw_error(path, "`workspaces.packages` must be an array"));
                return (true, Vec::new());
            };
            packages
        }
        _ => {
            errors.push(raw_error(
                path,
                "`workspaces` must be an array or an object with `packages`",
            ));
            return (true, Vec::new());
        }
    };

    let mut out = Vec::new();
    for (index, value) in packages.iter().enumerate() {
        if let Some(pattern) = value.as_str() {
            let pattern = pattern.trim();
            if pattern.is_empty() {
                errors.push(raw_error(
                    path,
                    format!("`workspaces` pattern at index {index} is empty"),
                ));
            } else {
                out.push(pattern.to_string());
            }
        } else {
            errors.push(raw_error(
                path,
                format!("`workspaces` pattern at index {index} must be a string"),
            ));
        }
    }
    (true, out)
}

/// Parse only the root `packages` sequence used by pnpm. Unsupported YAML
/// constructs become errors rather than being guessed or evaluated.
pub(crate) fn parse_pnpm_workspace_yaml(source: &str) -> (Vec<String>, Vec<String>) {
    let mut patterns = Vec::new();
    let mut errors = Vec::new();
    let mut in_packages = false;
    let mut saw_packages = false;

    for (index, original) in source.lines().enumerate() {
        let line_number = index + 1;
        let without_comment = strip_yaml_comment(original);
        let trimmed = without_comment.trim();
        if trimmed.is_empty() || trimmed == "---" || trimmed == "..." {
            continue;
        }
        let indentation = without_comment.len() - without_comment.trim_start().len();

        if indentation == 0 {
            if in_packages && (trimmed == "-" || trimmed.starts_with("- ")) {
                let item = trimmed.strip_prefix('-').unwrap_or(trimmed).trim();
                match parse_yaml_scalar(item) {
                    Ok(pattern) => patterns.push(pattern),
                    Err(message) => errors.push(format!("line {line_number}: {message}")),
                }
                continue;
            }
            if let Some(rest) = trimmed.strip_prefix("packages:") {
                if saw_packages {
                    errors.push(format!("duplicate `packages` key at line {line_number}"));
                }
                saw_packages = true;
                in_packages = true;
                let rest = rest.trim();
                if rest.is_empty() {
                    continue;
                }
                if rest == "[]" {
                    in_packages = false;
                    continue;
                }
                match parse_inline_yaml_list(rest) {
                    Ok(items) => patterns.extend(items),
                    Err(message) => errors.push(format!("line {line_number}: {message}")),
                }
                in_packages = false;
                continue;
            }
            in_packages = false;
            continue;
        }

        if in_packages {
            let Some(item) = trimmed.strip_prefix('-') else {
                errors.push(format!(
                    "line {line_number}: expected a `- <workspace glob>` list item"
                ));
                continue;
            };
            match parse_yaml_scalar(item.trim()) {
                Ok(pattern) => patterns.push(pattern),
                Err(message) => errors.push(format!("line {line_number}: {message}")),
            }
        }
    }

    if !saw_packages {
        errors.push("missing root `packages` key".to_string());
    }
    patterns.sort();
    patterns.dedup();
    (patterns, errors)
}

fn strip_yaml_comment(line: &str) -> String {
    let mut single = false;
    let mut double = false;
    let mut escaped = false;
    for (index, ch) in line.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match ch {
            '\\' if double => escaped = true,
            '\'' if !double => single = !single,
            '"' if !single => double = !double,
            '#' if !single && !double => return line[..index].to_string(),
            _ => {}
        }
    }
    line.to_string()
}

fn parse_inline_yaml_list(value: &str) -> Result<Vec<String>, String> {
    let Some(inner) = value.strip_prefix('[').and_then(|v| v.strip_suffix(']')) else {
        return Err("only a YAML list is supported after `packages:`".to_string());
    };
    if inner.trim().is_empty() {
        return Ok(Vec::new());
    }
    let mut items = Vec::new();
    let mut current = String::new();
    let mut single = false;
    let mut double = false;
    let mut escaped = false;
    for ch in inner.chars() {
        if escaped {
            current.push(ch);
            escaped = false;
            continue;
        }
        match ch {
            '\\' if double => {
                current.push(ch);
                escaped = true;
            }
            '\'' if !double => {
                single = !single;
                current.push(ch);
            }
            '"' if !single => {
                double = !double;
                current.push(ch);
            }
            ',' if !single && !double => {
                items.push(parse_yaml_scalar(current.trim())?);
                current.clear();
            }
            _ => current.push(ch),
        }
    }
    if single || double {
        return Err("unterminated quoted scalar".to_string());
    }
    items.push(parse_yaml_scalar(current.trim())?);
    Ok(items)
}

fn parse_yaml_scalar(value: &str) -> Result<String, String> {
    let value = value.trim().trim_end_matches(',').trim();
    if value.is_empty() {
        return Err("workspace glob is empty".to_string());
    }
    if value.starts_with('\'') {
        let Some(inner) = value.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')) else {
            return Err("unterminated single-quoted workspace glob".to_string());
        };
        let parsed = inner.replace("''", "'");
        if parsed.is_empty() {
            return Err("workspace glob is empty".to_string());
        }
        return Ok(parsed);
    }
    if value.starts_with('"') {
        return serde_json::from_str::<String>(value)
            .map_err(|error| format!("invalid double-quoted workspace glob: {error}"))
            .and_then(|parsed| {
                if parsed.is_empty() {
                    Err("workspace glob is empty".to_string())
                } else {
                    Ok(parsed)
                }
            });
    }
    if value.starts_with('[') || value.starts_with('{') || value.contains("\n") {
        return Err("unsupported YAML workspace glob construct".to_string());
    }
    Ok(value.to_string())
}

fn expand_workspace_patterns(
    patterns: &[String],
    package_dirs: &[String],
) -> (Vec<String>, Vec<String>, Vec<String>) {
    let mut included = BTreeSet::new();
    let mut excluded = BTreeSet::new();
    let mut unmatched = Vec::new();
    let mut errors = Vec::new();

    for original in patterns {
        let (negative, raw) = original
            .strip_prefix('!')
            .map_or((false, original.as_str()), |value| (true, value));
        let normalized = match normalize_workspace_pattern(raw) {
            Ok(pattern) => pattern,
            Err(message) => {
                errors.push(format!("invalid workspace glob `{original}`: {message}"));
                continue;
            }
        };
        let options = glob::MatchOptions {
            require_literal_separator: true,
            ..glob::MatchOptions::default()
        };
        let mut matches = BTreeSet::new();
        for expanded in expand_braces(&normalized) {
            let pattern = match glob::Pattern::new(&expanded) {
                Ok(pattern) => pattern,
                Err(error) => {
                    errors.push(format!("invalid workspace glob `{original}`: {error}"));
                    continue;
                }
            };
            matches.extend(
                package_dirs
                    .iter()
                    .filter(|dir| pattern.matches_with(&display_dir(dir), options))
                    .cloned(),
            );
        }
        if matches.is_empty() && !negative {
            unmatched.push(original.clone());
        }
        if negative {
            excluded.extend(matches);
        } else {
            included.extend(matches);
        }
    }

    for dir in excluded {
        included.remove(&dir);
    }
    let members = included.into_iter().map(|dir| display_dir(&dir)).collect();
    unmatched.sort();
    unmatched.dedup();
    errors.sort();
    errors.dedup();
    (members, unmatched, errors)
}

fn normalize_workspace_pattern(pattern: &str) -> Result<String, String> {
    let mut pattern = pattern.trim().replace('\\', "/");
    while let Some(stripped) = pattern.strip_prefix("./") {
        pattern = stripped.to_string();
    }
    pattern = pattern
        .trim_start_matches('/')
        .trim_end_matches('/')
        .to_string();
    if let Some(stripped) = pattern.strip_suffix("/package.json") {
        pattern = stripped.to_string();
    }
    if pattern.is_empty() {
        return Err("empty pattern".to_string());
    }
    if pattern.split('/').any(|part| part == "..") {
        return Err("pattern escapes the workspace root".to_string());
    }
    Ok(pattern)
}

pub(crate) fn workspace_pattern_matches(pattern: &str, rel: &str) -> bool {
    let Ok(normalized) = normalize_workspace_pattern(pattern) else {
        return false;
    };
    let options = glob::MatchOptions {
        require_literal_separator: true,
        ..glob::MatchOptions::default()
    };
    expand_braces(&normalized).into_iter().any(|expanded| {
        glob::Pattern::new(&expanded).is_ok_and(|pattern| pattern.matches_with(rel, options))
    })
}

fn expand_braces(pattern: &str) -> Vec<String> {
    let Some(open) = pattern.find('{') else {
        return vec![pattern.to_string()];
    };
    let Some(relative_close) = pattern[open + 1..].find('}') else {
        return vec![pattern.to_string()];
    };
    let close = open + 1 + relative_close;
    let alternatives: Vec<&str> = pattern[open + 1..close].split(',').collect();
    if alternatives.len() < 2 {
        return vec![pattern.to_string()];
    }
    let mut expanded = Vec::new();
    for alternative in alternatives {
        let next = format!(
            "{}{}{}",
            &pattern[..open],
            alternative,
            &pattern[close + 1..]
        );
        expanded.extend(expand_braces(&next));
    }
    expanded
}

fn same_root_lockfiles(fs: &FileSet, dir: &str) -> Vec<String> {
    let mut out: Vec<String> = LOCKFILE_NAMES
        .iter()
        .map(|(name, _)| join(dir, name))
        .filter(|path| fs.contains(path))
        .collect();
    out.sort();
    out
}

fn same_root_config_files(fs: &FileSet, dir: &str) -> Vec<String> {
    let mut out = Vec::new();
    for path in direct_files(fs, dir) {
        let name = path.rsplit('/').next().unwrap_or(&path);
        let language_config = name == "tsconfig.json" || name == "jsconfig.json";
        let framework_config = CONFIG_PREFIXES.iter().any(|prefix| {
            CONFIG_EXTENSIONS
                .iter()
                .any(|extension| name == format!("{prefix}.config.{extension}"))
        });
        if language_config || framework_config {
            out.push(path);
        }
    }
    out.sort();
    out
}

fn infer_package_manager(
    fs: &FileSet,
    dir: &str,
    manifests: &BTreeMap<String, PackageManifest>,
) -> (Option<RawPackageManager>, Vec<String>, Vec<String>) {
    let ancestors = ancestors_inclusive(dir);

    for ancestor in &ancestors {
        let Some(raw) = manifests
            .get(ancestor)
            .and_then(|manifest| manifest.package_manager.as_deref())
        else {
            continue;
        };
        if let Some(manager) = known_manager_from_package_manager(raw) {
            let source = join(ancestor, "package.json");
            return (
                Some(RawPackageManager {
                    name: manager.to_string(),
                    source: source.clone(),
                    explicit: true,
                }),
                vec![manager.to_string()],
                vec![format!("{source}:packageManager={raw}")],
            );
        }
    }

    for ancestor in ancestors {
        let evidence = manager_evidence_at(fs, &ancestor);
        if evidence.is_empty() {
            continue;
        }
        let candidates: Vec<String> = evidence.keys().cloned().collect();
        let details: Vec<String> = evidence.values().flatten().cloned().collect();
        if candidates.len() == 1 {
            let name = candidates[0].clone();
            let source = details
                .first()
                .cloned()
                .unwrap_or_else(|| display_dir(&ancestor));
            return (
                Some(RawPackageManager {
                    name,
                    source,
                    explicit: false,
                }),
                candidates,
                details,
            );
        }
        // Conflicting evidence at the nearest evidence-bearing directory is
        // intentionally not resolved using a farther-away ancestor.
        return (None, candidates, details);
    }

    (None, Vec::new(), Vec::new())
}

fn manager_evidence_at(fs: &FileSet, dir: &str) -> BTreeMap<String, Vec<String>> {
    let mut evidence: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (file, manager) in LOCKFILE_NAMES {
        let path = join(dir, file);
        if fs.contains(&path) {
            evidence
                .entry((*manager).to_string())
                .or_default()
                .push(path);
        }
    }
    let pnpm_workspace = join(dir, "pnpm-workspace.yaml");
    if fs.contains(&pnpm_workspace) {
        evidence
            .entry("pnpm".to_string())
            .or_default()
            .push(pnpm_workspace);
    }
    for values in evidence.values_mut() {
        values.sort();
    }
    evidence
}

fn known_manager_from_package_manager(raw: &str) -> Option<&'static str> {
    let name = raw.trim().split('@').next()?.to_ascii_lowercase();
    match name.as_str() {
        "npm" => Some("npm"),
        "pnpm" => Some("pnpm"),
        "yarn" => Some("yarn"),
        "bun" => Some("bun"),
        _ => None,
    }
}

fn classify_language(
    fs: &FileSet,
    dir: &str,
    owned_source_files: &[String],
    manifest: &PackageManifest,
) -> RawLanguageSignals {
    let mut typescript_evidence = BTreeSet::new();
    let mut javascript_evidence = BTreeSet::new();
    let tsconfig = join(dir, "tsconfig.json");
    let jsconfig = join(dir, "jsconfig.json");
    if fs.contains(&tsconfig) {
        typescript_evidence.insert(tsconfig);
    }
    if fs.contains(&jsconfig) {
        javascript_evidence.insert(jsconfig);
    }
    if direct_dependency_evidence(manifest, "typescript").is_some() {
        typescript_evidence.insert("dependency:typescript".to_string());
    }

    for path in owned_source_files {
        let lower = path.to_ascii_lowercase();
        if [".ts", ".tsx", ".mts", ".cts"]
            .iter()
            .any(|extension| lower.ends_with(extension))
        {
            typescript_evidence.insert(path.clone());
        } else if [".js", ".jsx", ".mjs", ".cjs"]
            .iter()
            .any(|extension| lower.ends_with(extension))
        {
            javascript_evidence.insert(path.clone());
        }
    }

    let typescript = !typescript_evidence.is_empty();
    let javascript = !javascript_evidence.is_empty();
    let primary = if typescript {
        Some("typescript".to_string())
    } else if javascript {
        Some("javascript".to_string())
    } else {
        None
    };
    let typescript_evidence: Vec<String> = typescript_evidence.into_iter().collect();
    let javascript_evidence: Vec<String> = javascript_evidence.into_iter().collect();
    let mut evidence: Vec<String> = typescript_evidence
        .iter()
        .chain(&javascript_evidence)
        .take(32)
        .cloned()
        .collect();
    evidence.sort();

    RawLanguageSignals {
        typescript,
        javascript,
        primary,
        evidence,
        typescript_evidence,
        javascript_evidence,
    }
}

fn classify_technologies(
    fs: &FileSet,
    dir: &str,
    manifest: &PackageManifest,
    config_files: &[String],
    has_index_html: bool,
    language: &RawLanguageSignals,
) -> (Vec<RawTechnologySignal>, RawViteSignals, RawInertiaSignals) {
    let mut signals: BTreeMap<String, (String, BTreeSet<String>)> = BTreeMap::new();

    for (id, package) in [
        ("nextjs", "next"),
        ("astro", "astro"),
        ("nuxt", "nuxt"),
        ("sveltekit", "@sveltejs/kit"),
        ("solidstart", "@solidjs/start"),
        ("remix", "@remix-run/dev"),
    ] {
        if let Some(evidence) = direct_dependency_evidence(manifest, package) {
            add_signal(&mut signals, id, "framework", evidence);
        }
    }

    for package in [
        "@tanstack/react-start",
        "@tanstack/solid-start",
        "@tanstack/start",
    ] {
        if let Some(evidence) = direct_dependency_evidence(manifest, package) {
            add_signal(&mut signals, "tanstack-start", "framework", evidence);
        }
    }
    for package in ["react", "vue", "svelte", "solid-js"] {
        if let Some(evidence) = direct_dependency_evidence(manifest, package) {
            add_signal(&mut signals, package, "ui", evidence);
        }
    }

    for evidence in &language.typescript_evidence {
        add_signal(&mut signals, "typescript", "language", evidence.clone());
    }
    for evidence in &language.javascript_evidence {
        add_signal(&mut signals, "javascript", "language", evidence.clone());
    }

    let react_router_dependency = direct_dependency_evidence(manifest, "@react-router/dev");
    let react_router_configs: Vec<String> = config_files
        .iter()
        .filter(|path| file_name(path).starts_with("react-router.config."))
        .cloned()
        .collect();
    let react_router_vite_configs: Vec<String> = config_files
        .iter()
        .filter(|path| file_name(path).starts_with("vite.config."))
        .filter(|path| {
            fs.read_str(path)
                .is_some_and(|source| source.contains("@react-router/dev/vite"))
        })
        .cloned()
        .collect();
    let react_router_build = manifest
        .scripts
        .get("build")
        .is_some_and(|script| command_invokes_subcommand(script, "react-router", "build"));
    if let Some(dependency_evidence) = react_router_dependency {
        if !react_router_configs.is_empty()
            || !react_router_vite_configs.is_empty()
            || react_router_build
        {
            add_signal(
                &mut signals,
                "react-router",
                "framework",
                dependency_evidence,
            );
            for path in react_router_configs
                .into_iter()
                .chain(react_router_vite_configs)
            {
                add_signal(
                    &mut signals,
                    "react-router",
                    "framework",
                    format!("config:{path}"),
                );
            }
            if react_router_build {
                add_signal(
                    &mut signals,
                    "react-router",
                    "framework",
                    "script:build".to_string(),
                );
            }
        }
    }

    let direct_vite_evidence = direct_dependency_evidence(manifest, "vite");
    let vite_config_files: Vec<String> = config_files
        .iter()
        .filter(|path| file_name(path).starts_with("vite.config."))
        .cloned()
        .collect();
    let vite_script_names: Vec<String> = manifest
        .scripts
        .iter()
        .filter(|(_, script)| command_invokes(script, "vite"))
        .map(|(name, _)| name.clone())
        .collect();
    let build_script_invokes_vite_build = manifest
        .scripts
        .get("build")
        .is_some_and(|script| command_invokes_subcommand(script, "vite", "build"));
    let direct_vite = direct_vite_evidence.is_some();
    let standalone = direct_vite && build_script_invokes_vite_build && has_index_html;
    if let Some(evidence) = direct_vite_evidence {
        add_signal(&mut signals, "vite", "build-tool", evidence);
    }
    for path in &vite_config_files {
        add_signal(&mut signals, "vite", "build-tool", format!("config:{path}"));
    }
    for script in &vite_script_names {
        add_signal(
            &mut signals,
            "vite",
            "build-tool",
            format!("script:{script}"),
        );
    }
    if standalone {
        add_signal(
            &mut signals,
            "vite",
            "build-tool",
            format!("marker:{}", join(dir, "index.html")),
        );
        add_signal(
            &mut signals,
            "vite",
            "build-tool",
            "qualification:standalone".to_string(),
        );
    }
    let vite = RawViteSignals {
        direct_dependency: direct_vite,
        config_files: vite_config_files,
        script_names: vite_script_names,
        build_script_invokes_vite_build,
        has_index_html,
        standalone,
    };

    let mut inertia = RawInertiaSignals::default();
    for (package, adapter) in [
        ("@inertiajs/react", Some("react")),
        ("@inertiajs/vue3", Some("vue3")),
        ("@inertiajs/svelte", Some("svelte")),
        ("@inertiajs/vite", None),
    ] {
        if let Some(evidence) = direct_dependency_evidence(manifest, package) {
            inertia.packages.push(package.to_string());
            if let Some(adapter) = adapter {
                inertia.adapters.push(adapter.to_string());
            } else {
                inertia.vite_helper = true;
            }
            add_signal(&mut signals, "inertia", "integration", evidence);
        }
    }
    inertia.adapters.sort();
    inertia.adapters.dedup();
    inertia.packages.sort();
    inertia.packages.dedup();
    inertia.corroborated = !inertia.packages.is_empty();

    let technologies = signals
        .into_iter()
        .map(|(id, (kind, evidence))| RawTechnologySignal {
            id,
            kind,
            evidence: evidence.into_iter().collect(),
        })
        .collect();
    (technologies, vite, inertia)
}

fn add_signal(
    signals: &mut BTreeMap<String, (String, BTreeSet<String>)>,
    id: &str,
    kind: &str,
    evidence: String,
) {
    let entry = signals
        .entry(id.to_string())
        .or_insert_with(|| (kind.to_string(), BTreeSet::new()));
    entry.1.insert(evidence);
}

fn direct_dependency_evidence(manifest: &PackageManifest, wanted: &str) -> Option<String> {
    for (group, values) in [
        ("dependencies", &manifest.dependencies),
        ("devDependencies", &manifest.dev_dependencies),
        ("optionalDependencies", &manifest.optional_dependencies),
    ] {
        if let Some((name, version)) = values
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(wanted))
        {
            return Some(format!("{group}:{name}@{version}"));
        }
    }
    None
}

fn command_invokes(script: &str, command: &str) -> bool {
    command_occurrences(script, command).next().is_some()
}

fn command_invokes_subcommand(script: &str, command: &str, subcommand: &str) -> bool {
    command_occurrences(script, command).any(|(tokens, command_index)| {
        let mut index = command_index + 1;
        while index < tokens.len() && !is_shell_operator(&tokens[index]) {
            let token = &tokens[index];
            if token == "--" {
                index += 1;
                return tokens.get(index).is_some_and(|token| token == subcommand);
            }
            if token.starts_with('-') {
                if option_takes_value(command, token) && !token.contains('=') {
                    index += 1;
                }
                index += 1;
                continue;
            }
            return token == subcommand;
        }
        false
    })
}

fn option_takes_value(command: &str, option: &str) -> bool {
    match command {
        "vite" => matches!(
            option,
            "--config" | "-c" | "--base" | "--mode" | "-m" | "--logLevel" | "--host" | "--port"
        ),
        "react-router" => matches!(option, "--config" | "-c" | "--mode" | "-m"),
        _ => false,
    }
}

fn command_occurrences<'a>(
    script: &'a str,
    command: &'a str,
) -> impl Iterator<Item = (Vec<String>, usize)> + 'a {
    split_shell_segments(script)
        .into_iter()
        .filter_map(move |segment| {
            let tokens = shell_words(&segment);
            recognized_command_index(&tokens, command).map(|index| (tokens, index))
        })
}

fn recognized_command_index(tokens: &[String], command: &str) -> Option<usize> {
    let mut index = 0;
    while index < tokens.len() && is_environment_assignment(&tokens[index]) {
        index += 1;
    }
    let first_executable = tokens.get(index).map(|token| executable_name(token));
    if matches!(
        first_executable.as_deref(),
        Some("cross-env" | "cross-env-shell")
    ) {
        index += 1;
        while index < tokens.len()
            && (tokens[index].starts_with('-') || is_environment_assignment(&tokens[index]))
        {
            index += 1;
        }
    }
    let executable = tokens.get(index).map(|token| executable_name(token))?;
    if executable == command {
        return Some(index);
    }

    match executable.as_str() {
        "npx" | "bunx" => find_wrapped_command(tokens, index + 1, command, &[]),
        "pnpm" | "yarn" => find_wrapped_command(tokens, index + 1, command, &["exec", "dlx", "x"]),
        "npm" => {
            let tail = &tokens[index + 1..];
            if tail
                .iter()
                .any(|token| matches!(token.as_str(), "exec" | "x"))
            {
                find_wrapped_command(tokens, index + 1, command, &["exec", "x"])
            } else {
                None
            }
        }
        "bun" => find_wrapped_command(tokens, index + 1, command, &["x"]),
        "node" => tokens
            .iter()
            .enumerate()
            .skip(index + 1)
            .find_map(|(i, token)| {
                let normalized = token.replace('\\', "/");
                (normalized.contains(&format!("/{command}/"))
                    || normalized.ends_with(&format!("/{command}")))
                .then_some(i)
            }),
        _ => None,
    }
}

fn find_wrapped_command(
    tokens: &[String],
    start: usize,
    command: &str,
    ignorable_words: &[&str],
) -> Option<usize> {
    tokens
        .iter()
        .enumerate()
        .skip(start)
        .find_map(|(index, token)| {
            let executable = executable_name(token);
            if token.starts_with('-')
                || token == "--"
                || ignorable_words.contains(&executable.as_str())
                || is_environment_assignment(token)
            {
                None
            } else if executable == command {
                Some(index)
            } else {
                // A different executable means a wrapper such as `npm run foo`;
                // do not treat later mentions as command execution.
                Some(usize::MAX)
            }
        })
        .filter(|index| *index != usize::MAX)
}

fn split_shell_segments(script: &str) -> Vec<String> {
    let mut segments = Vec::new();
    let mut current = String::new();
    let mut single = false;
    let mut double = false;
    let mut escaped = false;
    for ch in script.chars() {
        if escaped {
            current.push(ch);
            escaped = false;
            continue;
        }
        match ch {
            '\\' if !single => escaped = true,
            '\'' if !double => {
                single = !single;
                current.push(ch);
            }
            '"' if !single => {
                double = !double;
                current.push(ch);
            }
            ';' | '&' | '|' if !single && !double => {
                if !current.trim().is_empty() {
                    segments.push(current.trim().to_string());
                }
                current.clear();
            }
            _ => current.push(ch),
        }
    }
    if !current.trim().is_empty() {
        segments.push(current.trim().to_string());
    }
    segments
}

fn shell_words(segment: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut single = false;
    let mut double = false;
    let mut escaped = false;
    for ch in segment.chars() {
        if escaped {
            current.push(ch);
            escaped = false;
            continue;
        }
        match ch {
            '\\' if !single => escaped = true,
            '\'' if !double => single = !single,
            '"' if !single => double = !double,
            ch if ch.is_whitespace() && !single && !double => {
                if !current.is_empty() {
                    words.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(ch),
        }
    }
    if !current.is_empty() {
        words.push(current);
    }
    words
}

fn executable_name(token: &str) -> String {
    let name = token
        .replace('\\', "/")
        .rsplit('/')
        .next()
        .unwrap_or(token)
        .to_ascii_lowercase();
    for suffix in [".cmd", ".exe", ".ps1"] {
        if let Some(stripped) = name.strip_suffix(suffix) {
            return stripped.to_string();
        }
    }
    name
}

fn is_environment_assignment(token: &str) -> bool {
    token
        .split_once('=')
        .is_some_and(|(name, _)| !name.is_empty() && !name.contains('/'))
}

fn is_shell_operator(token: &str) -> bool {
    matches!(token, ";" | "&" | "&&" | "|" | "||")
}

fn source_files_by_owner(
    fs: &FileSet,
    boundaries: &BTreeSet<String>,
) -> BTreeMap<String, Vec<String>> {
    let mut owned: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for path in fs.files.keys() {
        let mut directory = path
            .rsplit_once('/')
            .map(|(parent, _)| parent)
            .unwrap_or("");
        loop {
            if boundaries.contains(directory) {
                owned
                    .entry(directory.to_string())
                    .or_default()
                    .push(path.clone());
                break;
            }
            let Some((parent, _)) = directory.rsplit_once('/') else {
                if boundaries.contains("") {
                    owned.entry(String::new()).or_default().push(path.clone());
                }
                break;
            };
            directory = parent;
        }
    }
    owned
}

fn direct_files(fs: &FileSet, dir: &str) -> Vec<String> {
    let mut out: Vec<String> = fs
        .under(dir)
        .filter(|path| {
            let local = if dir.is_empty() {
                *path
            } else {
                path.strip_prefix(&format!("{dir}/")).unwrap_or(path)
            };
            !local.contains('/')
        })
        .map(str::to_string)
        .collect();
    out.sort();
    out
}

fn ancestors_inclusive(dir: &str) -> Vec<String> {
    let mut out = vec![dir.to_string()];
    let mut current = dir;
    while let Some((parent, _)) = current.rsplit_once('/') {
        out.push(parent.to_string());
        current = parent;
    }
    if !out.iter().any(String::is_empty) {
        out.push(String::new());
    }
    out
}

fn join(dir: &str, name: &str) -> String {
    if dir.is_empty() || dir == "." {
        name.to_string()
    } else {
        format!("{dir}/{name}")
    }
}

fn display_dir(dir: &str) -> String {
    if dir.is_empty() {
        ".".to_string()
    } else {
        dir.to_string()
    }
}

fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn raw_error(path: &str, message: impl Into<String>) -> RawNodeParseError {
    RawNodeParseError {
        path: path.to_string(),
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed_manifest(source: &str) -> (PackageManifest, Vec<RawNodeParseError>) {
        let mut errors = Vec::new();
        let manifest = parse_package_json("package.json", source, &mut errors);
        (manifest, errors)
    }

    #[test]
    fn parses_package_json_fields_and_workspace_shapes() {
        let (manifest, errors) = parsed_manifest(
            r#"{
                "name": "web",
                "packageManager": "pnpm@10.12.1",
                "dependencies": {"next": "16.0.0", "react": "19.0.0"},
                "devDependencies": {"vite": "7.0.0"},
                "optionalDependencies": {"typescript": "5.9.0"},
                "scripts": {"build": "vite build"},
                "engines": {"node": ">=20"},
                "workspaces": {"packages": ["apps/*", "packages/*"]}
            }"#,
        );
        assert!(errors.is_empty());
        assert!(manifest.parsed);
        assert_eq!(manifest.name.as_deref(), Some("web"));
        assert_eq!(manifest.package_manager.as_deref(), Some("pnpm@10.12.1"));
        assert!(manifest.dependencies.contains_key("next"));
        assert!(manifest.dev_dependencies.contains_key("vite"));
        assert!(manifest.optional_dependencies.contains_key("typescript"));
        assert_eq!(manifest.requires_node.as_deref(), Some(">=20"));
        assert_eq!(manifest.workspace_patterns, ["apps/*", "packages/*"]);
    }

    #[test]
    fn malformed_package_fields_degrade_to_errors_and_keep_other_facts() {
        let (manifest, errors) = parsed_manifest(
            r#"{
                "name": "web",
                "dependencies": {"astro": "5", "broken": 3},
                "scripts": [],
                "workspaces": ["apps/*", 42]
            }"#,
        );
        assert!(manifest.parsed);
        assert!(manifest.dependencies.contains_key("astro"));
        assert!(!manifest.dependencies.contains_key("broken"));
        assert_eq!(manifest.workspace_patterns, ["apps/*"]);
        assert_eq!(errors.len(), 3);
    }

    #[test]
    fn parses_conservative_pnpm_workspace_yaml() {
        let (patterns, errors) = parse_pnpm_workspace_yaml(
            r#"
packages:
  - 'apps/*'
  - "packages/**"
  - '!packages/fixtures/**' # not deployable packages
catalog:
  react: ^19
"#,
        );
        assert!(errors.is_empty());
        assert_eq!(patterns, ["!packages/fixtures/**", "apps/*", "packages/**"]);

        let (inline, errors) = parse_pnpm_workspace_yaml("packages: ['a/*', \"b/*\"]\n");
        assert!(errors.is_empty());
        assert_eq!(inline, ["a/*", "b/*"]);
    }

    #[test]
    fn workspace_expansion_is_sorted_and_exclusions_win() {
        let patterns = vec![
            "packages/**".to_string(),
            "apps/*".to_string(),
            "!packages/fixtures/**".to_string(),
        ];
        let dirs = vec![
            String::new(),
            "apps/web".to_string(),
            "packages/core".to_string(),
            "packages/fixtures/demo".to_string(),
        ];
        let (members, unmatched, errors) = expand_workspace_patterns(&patterns, &dirs);
        assert!(unmatched.is_empty());
        assert!(errors.is_empty());
        assert_eq!(members, ["apps/web", "packages/core"]);
    }

    #[test]
    fn filesystem_walk_keeps_library_workspace_members() {
        let fixture = fixture_fileset(&[
            (
                ".gitignore",
                "node_modules/\ndist/\nbuild/\n.next/\ncoverage/\n",
            ),
            (
                "package.json",
                r#"{"private":true,"workspaces":["apps/*","packages/*"]}"#,
            ),
            ("apps/web/package.json", r#"{"dependencies":{"next":"16"}}"#),
            ("packages/lib/package.json", r#"{"name":"@fixture/lib"}"#),
        ]);
        let walked = crate::fileset::walk_fs(&fixture.root, None, false, &[]);
        let discovery = discover(&walked);
        assert_eq!(
            discovery.workspaces[0].members,
            ["apps/web", "packages/lib"]
        );
        let _ = std::fs::remove_dir_all(&fixture.root);
    }

    #[test]
    fn command_detection_is_conservative_and_understands_wrappers() {
        assert!(command_invokes_subcommand(
            "tsc -b && vite build --mode production",
            "vite",
            "build"
        ));
        assert!(command_invokes_subcommand(
            "cross-env NODE_ENV=production pnpm exec vite build",
            "vite",
            "build"
        ));
        assert!(command_invokes_subcommand(
            "react-router build",
            "react-router",
            "build"
        ));
        assert!(!command_invokes("echo vite", "vite"));
        assert!(!command_invokes("vitest run", "vite"));
        assert!(!command_invokes("npm run build-vite", "vite"));
        assert!(!command_invokes("yarn run vite", "vite"));
    }

    #[test]
    fn strict_vite_and_framework_signals_are_distinct() {
        let (manifest, errors) = parsed_manifest(
            r#"{
                "dependencies": {
                    "astro": "5",
                    "react": "19",
                    "@inertiajs/react": "2"
                },
                "devDependencies": {"vite": "7"},
                "scripts": {"build": "vite build"}
            }"#,
        );
        assert!(errors.is_empty());
        let language = RawLanguageSignals::default();
        let (signals, vite, inertia) =
            classify_technologies(&unreadable_fileset(), "", &manifest, &[], true, &language);
        let ids: Vec<&str> = signals.iter().map(|signal| signal.id.as_str()).collect();
        assert!(ids.contains(&"astro"));
        assert!(ids.contains(&"react"));
        assert!(ids.contains(&"vite"));
        assert!(ids.contains(&"inertia"));
        assert!(vite.standalone);
        assert!(inertia.corroborated);
        assert_eq!(inertia.adapters, ["react"]);
    }

    #[test]
    fn discovery_combines_packages_workspaces_and_nearest_manager_facts() {
        let fs = fixture_fileset(&[
            (
                "package.json",
                r#"{
                    "name": "repo",
                    "private": true,
                    "packageManager": "pnpm@10.12.1",
                    "workspaces": ["apps/*"]
                }"#,
            ),
            ("pnpm-lock.yaml", "lockfileVersion: '9.0'\n"),
            (
                "pnpm-workspace.yaml",
                "packages:\n  - 'apps/*'\n  - 'packages/*'\n",
            ),
            (
                "apps/web/package.json",
                r#"{
                    "name": "web",
                    "dependencies": {"next": "16", "react": "19"},
                    "devDependencies": {"vite": "7", "typescript": "5"},
                    "scripts": {"build": "vite build"}
                }"#,
            ),
            ("apps/web/package-lock.json", "{}\n"),
            ("apps/web/index.html", "<!doctype html>\n"),
            ("apps/web/tsconfig.json", "{}\n"),
            ("apps/web/src/main.tsx", "export {}\n"),
            ("packages/broken/package.json", "{not json\n"),
        ]);

        let discovery = discover(&fs);
        assert_eq!(
            discovery
                .packages
                .iter()
                .map(|package| package.path.as_str())
                .collect::<Vec<_>>(),
            [".", "apps/web", "packages/broken"]
        );
        let web = discovery
            .packages
            .iter()
            .find(|package| package.path == "apps/web")
            .expect("web package");
        assert_eq!(
            web.package_manager
                .as_ref()
                .map(|manager| manager.name.as_str()),
            Some("pnpm")
        );
        assert!(web
            .package_manager
            .as_ref()
            .is_some_and(|manager| manager.explicit));
        assert_eq!(web.language.primary.as_deref(), Some("typescript"));
        assert!(web.vite.standalone);
        assert!(web.technologies.iter().any(|signal| signal.id == "nextjs"));

        assert_eq!(discovery.workspaces.len(), 1);
        assert_eq!(
            discovery.workspaces[0].members,
            ["apps/web", "packages/broken"]
        );
        assert_eq!(discovery.workspaces[0].sources.len(), 2);
        assert!(discovery
            .parse_errors
            .iter()
            .any(|error| error.path == "packages/broken/package.json"
                && error.message.starts_with("invalid JSON:")));

        let _ = std::fs::remove_dir_all(&fs.root);
    }

    fn fixture_fileset(files: &[(&str, &str)]) -> FileSet {
        use std::sync::atomic::{AtomicU64, Ordering};

        static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);
        let suffix = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
        let root =
            std::env::temp_dir().join(format!("kenbun-node-test-{}-{suffix}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("create fixture root");
        let mut entries = BTreeMap::new();
        for (path, source) in files {
            let absolute = root.join(path);
            if let Some(parent) = absolute.parent() {
                std::fs::create_dir_all(parent).expect("create fixture parent");
            }
            std::fs::write(&absolute, source).expect("write fixture file");
            entries.insert((*path).to_string(), source.len() as u64);
        }
        FileSet::test_local(root, entries)
    }

    fn unreadable_fileset() -> FileSet {
        FileSet::test_local(std::path::PathBuf::new(), BTreeMap::new())
    }
}
