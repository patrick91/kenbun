//! Explicit Python framework detector orchestration.
//!
//! Each framework owns its dependency names and framework-specific analysis.
//! Calls remain concrete so their cost and ordering are visible here.

mod django;
mod fastapi;
mod flask;

use super::manifest::{self, PyProject};
use crate::diag;
use crate::fileset::FileSet;
use crate::model::{DeclaredDep, Diagnostic, Entrypoint, Evidence};

#[derive(Clone, Debug)]
pub(crate) struct DeployTarget {
    pub framework: String,
    pub entrypoint: Option<Entrypoint>,
    pub confidence: String,
    pub evidence: Vec<Evidence>,
    pub diagnostics: Vec<Diagnostic>,
}

pub(crate) struct Detection {
    pub frameworks: Vec<String>,
    pub deploy_targets: Vec<DeployTarget>,
    pub evidence: Vec<Evidence>,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Default)]
pub(super) struct DetectorResult {
    pub framework: Option<&'static str>,
    pub deploy_targets: Vec<DeployTarget>,
    pub evidence: Vec<Evidence>,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Clone, Copy)]
pub(super) struct DependencyLookup<'a> {
    dependencies: &'a [DeclaredDep],
}

impl<'a> DependencyLookup<'a> {
    fn new(dependencies: &'a [DeclaredDep]) -> Self {
        Self { dependencies }
    }

    /// Return the contiguous range for a normalized name in the sorted input.
    pub(super) fn named(self, name: &str) -> &'a [DeclaredDep] {
        let start = self
            .dependencies
            .partition_point(|dependency| dependency.name.as_str() < name);
        let end = start
            + self.dependencies[start..]
                .partition_point(|dependency| dependency.name.as_str() == name);
        &self.dependencies[start..end]
    }
}

struct SetupPy {
    path: String,
    mentions: Vec<String>,
}

pub(super) struct Context<'a> {
    pub fs: &'a FileSet,
    pub dir: &'a str,
    pub display_path: &'a str,
    pub dependencies: DependencyLookup<'a>,
    pub pyproject: Option<&'a PyProject>,
    pub package_manager: &'a str,
    pub entrypoint_hint: Option<&'a str>,
    setup_py: Option<SetupPy>,
}

impl Context<'_> {
    pub(super) fn setup_evidence(&self, package_names: &[&str]) -> Vec<Evidence> {
        let Some(setup_py) = &self.setup_py else {
            return Vec::new();
        };
        setup_py
            .mentions
            .iter()
            .filter(|name| package_names.contains(&name.as_str()))
            .map(|name| Evidence {
                kind: "marker-file".to_string(),
                path: setup_py.path.clone(),
                span: None,
                detail: format!("setup.py mentions {name} (string scan; not executed)"),
            })
            .collect()
    }
}

pub(crate) fn detect(
    fs: &FileSet,
    dir: &str,
    display_path: &str,
    dependencies: &[DeclaredDep],
    pyproject: Option<&PyProject>,
    package_manager: &str,
    entrypoint_hint: Option<&str>,
) -> Detection {
    let setup_py_path = join(dir, "setup.py");
    let setup_py = fs.read_str(&setup_py_path).map(|source| SetupPy {
        path: setup_py_path,
        mentions: manifest::setup_py_requirement_mentions(&source, |name| {
            django::matches_dependency(name)
                || fastapi::matches_dependency(name)
                || flask::matches_dependency(name)
        }),
    });
    let context = Context {
        fs,
        dir,
        display_path,
        dependencies: DependencyLookup::new(dependencies),
        pyproject,
        package_manager,
        entrypoint_hint,
        setup_py,
    };
    let mut detection = Detection {
        frameworks: Vec::new(),
        deploy_targets: Vec::new(),
        evidence: Vec::new(),
        diagnostics: Vec::new(),
    };

    detection.merge(django::detect(&context));
    detection.merge(fastapi::detect(&context));
    detection.merge(flask::detect(&context));

    detection.frameworks.sort();
    if detection.frameworks.len() > 1 {
        detection
            .diagnostics
            .push(diag::kb101(display_path, &detection.frameworks));
    }
    detection
}

impl Detection {
    fn merge(&mut self, result: DetectorResult) {
        if let Some(framework) = result.framework {
            self.frameworks.push(framework.to_string());
        }
        self.deploy_targets.extend(result.deploy_targets);
        self.evidence.extend(result.evidence);
        self.diagnostics.extend(result.diagnostics);
    }
}

pub(super) fn dependency_evidence(dependency: &DeclaredDep) -> Evidence {
    Evidence {
        kind: "dependency-declared".to_string(),
        path: dependency.source.path.clone(),
        span: None,
        detail: format!("{} in `{}`", dependency.raw, dependency.group),
    }
}

pub(super) fn join(directory: &str, name: &str) -> String {
    if directory.is_empty() || directory == "." {
        name.to_string()
    } else {
        format!("{directory}/{name}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::SourceRef;

    fn dependency(name: &str, path: &str) -> DeclaredDep {
        DeclaredDep {
            name: name.to_string(),
            raw: name.to_string(),
            specifier: String::new(),
            extras: Vec::new(),
            markers: None,
            group: "project".to_string(),
            source: SourceRef {
                path: path.to_string(),
                span: None,
            },
        }
    }

    #[test]
    fn dependency_lookup_returns_only_the_requested_sorted_range() {
        let dependencies = vec![
            dependency("django", "a.toml"),
            dependency("fastapi", "b.toml"),
            dependency("fastapi", "c.txt"),
            dependency("flask", "d.toml"),
        ];
        let lookup = DependencyLookup::new(&dependencies);

        assert_eq!(lookup.named("fastapi").len(), 2);
        assert_eq!(lookup.named("fastapi")[0].source.path, "b.toml");
        assert!(lookup.named("litestar").is_empty());
    }
}
