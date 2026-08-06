//! Raw Node/JavaScript project discovery.
//!
//! This module deliberately stops short of constructing public scan-model
//! objects.  It collects deterministic, non-executing facts that the root
//! scanner can later reconcile with Python (and other ecosystem) projects.

use std::collections::{BTreeMap, BTreeSet};

mod frameworks;
mod manifest;
mod workspace;

use super::boundary;
use super::runtime::{self, RuntimePin};
use crate::fileset::FileSet;
use manifest::{parse_package_json, PackageManifest};
use workspace::expand_workspace_patterns;

pub(crate) use workspace::{parse_pnpm_workspace_yaml, workspace_pattern_matches};

const LOCKFILE_NAMES: &[(&str, &str)] = &[
    ("package-lock.json", "npm"),
    ("npm-shrinkwrap.json", "npm"),
    ("pnpm-lock.yaml", "pnpm"),
    ("yarn.lock", "yarn"),
    ("bun.lock", "bun"),
    ("bun.lockb", "bun"),
];
const CONFIG_EXTENSIONS: &[&str] = &["js", "mjs", "cjs", "ts", "mts", "cts"];

type TechnologySignals = BTreeMap<String, (String, BTreeSet<String>)>;

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
    pub requires_node: Option<String>,
    pub version_pins: Vec<RuntimePin>,
    pub package_manager: Option<RawPackageManager>,
    /// Candidates at the nearest lock/workspace evidence level. More than one
    /// means the evidence was ambiguous and `package_manager` is None.
    pub package_manager_candidates: Vec<String>,
    pub declares_workspace: bool,
    pub workspace_patterns: Vec<String>,
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
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RawPackageManager {
    pub name: String,
    pub version: Option<String>,
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
    pub has_index_html: bool,
    /// A direct Vite dependency and same-root index.html distinguish a
    /// standalone application from a Vite-powered library or asset pipeline.
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
    let source_boundaries = boundary::project_directories(fs);
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
        let config_files = same_root_config_files(fs, dir);
        let index_html_path = join(dir, "index.html");
        let index_html = fs.contains(&index_html_path).then_some(index_html_path);
        let (package_manager, package_manager_candidates) =
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
            requires_node: manifest.requires_node.clone(),
            version_pins,
            package_manager,
            package_manager_candidates,
            declares_workspace: manifest.declares_workspace,
            workspace_patterns: manifest.workspace_patterns.clone(),
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
        let (package_manager, _) = infer_package_manager(fs, &dir, &manifests);
        workspaces.push(RawNodeWorkspace {
            path: display_dir(&dir),
            sources,
            patterns,
            members,
            unmatched_patterns: unmatched,
            has_root_package: manifests.contains_key(&dir),
            package_manager,
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

fn same_root_config_files(fs: &FileSet, dir: &str) -> Vec<String> {
    let mut out = Vec::new();
    for path in direct_files(fs, dir) {
        let name = path.rsplit('/').next().unwrap_or(&path);
        let language_config = name == "tsconfig.json" || name == "jsconfig.json";
        let framework_config = frameworks::is_config_file(name);
        let vite_config = CONFIG_EXTENSIONS
            .iter()
            .any(|extension| name == format!("vite.config.{extension}"));
        if language_config || framework_config || vite_config {
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
) -> (Option<RawPackageManager>, Vec<String>) {
    let ancestors = ancestors_inclusive(dir);

    for ancestor in &ancestors {
        let Some(raw) = manifests
            .get(ancestor)
            .and_then(|manifest| manifest.package_manager.as_deref())
        else {
            continue;
        };
        if let Some((manager, version)) = parse_package_manager(raw) {
            let source = join(ancestor, "package.json");
            return (
                Some(RawPackageManager {
                    name: manager.to_string(),
                    version,
                    source,
                    explicit: true,
                }),
                vec![manager.to_string()],
            );
        }
    }

    for ancestor in ancestors {
        let evidence = manager_evidence_at(fs, &ancestor);
        if evidence.is_empty() {
            continue;
        }
        let candidates: Vec<String> = evidence.keys().cloned().collect();
        if candidates.len() == 1 {
            let name = candidates[0].clone();
            let source = evidence[&name]
                .first()
                .cloned()
                .unwrap_or_else(|| display_dir(&ancestor));
            return (
                Some(RawPackageManager {
                    name,
                    version: None,
                    source,
                    explicit: false,
                }),
                candidates,
            );
        }
        return (None, candidates);
    }

    (None, Vec::new())
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

fn parse_package_manager(raw: &str) -> Option<(&'static str, Option<String>)> {
    let raw = raw.trim();
    let (name, version) = raw.split_once('@').unwrap_or((raw, ""));
    let name = name.to_ascii_lowercase();
    let version = (!version.is_empty()).then(|| version.to_string());
    match name.as_str() {
        "npm" => Some(("npm", version)),
        "pnpm" => Some(("pnpm", version)),
        "yarn" => Some(("yarn", version)),
        "bun" => Some(("bun", version)),
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
    dir: &str,
    manifest: &PackageManifest,
    config_files: &[String],
    has_index_html: bool,
    language: &RawLanguageSignals,
) -> (Vec<RawTechnologySignal>, RawViteSignals, RawInertiaSignals) {
    let mut signals = TechnologySignals::new();

    frameworks::detect(manifest, &mut signals);
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

    let direct_vite_evidence = direct_dependency_evidence(manifest, "vite");
    let vite_config_files: Vec<String> = config_files
        .iter()
        .filter(|path| file_name(path).starts_with("vite.config."))
        .cloned()
        .collect();
    let direct_vite = direct_vite_evidence.is_some();
    let standalone = direct_vite && has_index_html;
    if let Some(evidence) = direct_vite_evidence {
        add_signal(&mut signals, "vite", "build-tool", evidence);
    }
    for path in &vite_config_files {
        add_signal(&mut signals, "vite", "build-tool", format!("config:{path}"));
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

fn add_signal(signals: &mut TechnologySignals, id: &str, kind: &str, evidence: String) {
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
        let walked = crate::fileset::walk_fs(&fixture.root, None, false, &[], None);
        let discovery = discover(&walked);
        assert_eq!(
            discovery.workspaces[0].members,
            ["apps/web", "packages/lib"]
        );
        let _ = std::fs::remove_dir_all(&fixture.root);
    }

    #[test]
    fn vite_application_and_framework_signals_are_distinct() {
        let (manifest, errors) = parsed_manifest(
            r#"{
                "dependencies": {
                    "astro": "5",
                    "react": "19",
                    "@inertiajs/react": "2"
                },
                "devDependencies": {"vite": "7"}
            }"#,
        );
        assert!(errors.is_empty());
        let language = RawLanguageSignals::default();
        let (signals, vite, inertia) = classify_technologies("", &manifest, &[], true, &language);
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
}
