//! Public output model. Declaration order is the canonical JSON key order.

use pyo3::prelude::*;
use serde::Serialize;

pub const SCHEMA_VERSION: u32 = 2;

#[pyclass(get_all, frozen, skip_from_py_object)]
#[derive(Clone, Debug, Serialize)]
pub struct Span {
    pub start_line: u32,
    pub start_col: u32,
    pub end_line: u32,
    pub end_col: u32,
}

#[pyclass(get_all, frozen, skip_from_py_object)]
#[derive(Clone, Debug, Serialize)]
pub struct Diagnostic {
    pub code: String,
    pub severity: String,
    pub message: String,
    pub path: Option<String>,
    pub span: Option<Span>,
}

#[pyclass(get_all, frozen, skip_from_py_object)]
#[derive(Clone, Debug, Serialize)]
pub struct Evidence {
    pub kind: String,
    pub path: String,
    pub span: Option<Span>,
    pub detail: String,
}

#[pyclass(get_all, frozen, skip_from_py_object)]
#[derive(Clone, Debug, Serialize)]
pub struct Entrypoint {
    pub kind: String,
    pub module: String,
    pub attribute: String,
    pub is_factory: bool,
    pub import_root: String,
    pub source: String,
    pub as_string: String,
}

#[pyclass(get_all, frozen, skip_from_py_object)]
#[derive(Clone, Debug, Serialize)]
pub struct EnvVar {
    pub names: Vec<String>,
    pub required: bool,
    pub case_sensitive: bool,
    pub value_kind: String,
    pub has_default: bool,
    pub default_is_computed: bool,
    pub source: String,
    pub origin_path: String,
    pub origin_span: Option<Span>,
    pub origin_symbol: String,
    pub confidence: String,
}

#[pyclass(get_all, frozen, skip_from_py_object)]
#[derive(Clone, Debug, Serialize)]
pub struct SourceRef {
    pub path: String,
    pub span: Option<Span>,
}

#[pyclass(get_all, frozen, skip_from_py_object)]
#[derive(Clone, Debug, Serialize)]
pub struct DeclaredDep {
    pub name: String,
    pub raw: String,
    pub specifier: String,
    pub extras: Vec<String>,
    pub markers: Option<String>,
    pub group: String,
    pub source: SourceRef,
}

#[pyclass(get_all, frozen, skip_from_py_object)]
#[derive(Clone, Debug, Serialize)]
pub struct ResolvedDep {
    pub name: String,
    pub version: String,
    pub source: String,
    pub marker: Option<String>,
}

#[pyclass(get_all, frozen, skip_from_py_object)]
#[derive(Clone, Debug, Serialize)]
pub struct ManifestRef {
    pub path: String,
    pub kind: String,
}

#[pyclass(get_all, frozen, skip_from_py_object)]
#[derive(Clone, Debug, Serialize)]
pub struct LockfileRef {
    pub path: String,
    pub kind: String,
    pub parsed: bool,
}

#[pyclass(get_all, frozen, skip_from_py_object)]
#[derive(Clone, Debug, Serialize)]
pub struct DependencySet {
    pub ecosystem: String,
    pub package_manager: Option<String>,
    pub manifests: Vec<ManifestRef>,
    pub lockfiles: Vec<LockfileRef>,
    pub declared: Vec<DeclaredDep>,
    pub resolved: Vec<ResolvedDep>,
}

#[pyclass(get_all, frozen, skip_from_py_object)]
#[derive(Clone, Debug, Serialize)]
pub struct Technology {
    pub name: String,
    pub kind: String,
    pub role: String,
    pub confidence: String,
    pub evidence: Vec<Evidence>,
}

#[pyclass(get_all, frozen, skip_from_py_object)]
#[derive(Clone, Debug, Serialize)]
pub struct BuildScript {
    pub name: String,
    pub command: String,
    pub package_manager: Option<String>,
    pub argv: Option<Vec<String>>,
    pub source: SourceRef,
}

#[pyclass(get_all, frozen, skip_from_py_object)]
#[derive(Clone, Debug, Serialize)]
pub struct VersionPin {
    pub source: String,
    pub value: String,
}

#[pyclass(get_all, frozen, skip_from_py_object)]
#[derive(Clone, Debug, Serialize)]
pub struct PythonInfo {
    pub requires_python: Option<String>,
    pub version_pins: Vec<VersionPin>,
}

#[pyclass(get_all, frozen, skip_from_py_object)]
#[derive(Clone, Debug, Serialize)]
pub struct NodeInfo {
    pub requires_node: Option<String>,
    pub version_pins: Vec<VersionPin>,
}

#[derive(Clone, Debug)]
pub(crate) struct DeployTarget {
    pub framework: String,
    pub entrypoint: Option<Entrypoint>,
    pub confidence: String,
    pub evidence: Vec<Evidence>,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Clone, Debug)]
pub(crate) struct Project {
    pub path: String,
    pub name: Option<String>,
    pub is_python_project: bool,
    pub frameworks: Vec<String>,
    pub deploy_targets: Vec<DeployTarget>,
    pub dependencies: Option<DependencySet>,
    pub env_vars: Vec<EnvVar>,
    pub python: PythonInfo,
    pub evidence: Vec<Evidence>,
    pub diagnostics: Vec<Diagnostic>,
}

#[pyclass(get_all, frozen, skip_from_py_object)]
#[derive(Clone, Debug, Serialize)]
pub struct Application {
    pub application_dir: String,
    pub name: Option<String>,
    pub technologies: Vec<Technology>,
    pub entrypoint: Option<Entrypoint>,
    pub dependencies: Vec<DependencySet>,
    pub build_scripts: Vec<BuildScript>,
    pub env_vars: Vec<EnvVar>,
    pub python: Option<PythonInfo>,
    pub node: Option<NodeInfo>,
    pub evidence: Vec<Evidence>,
    pub diagnostics: Vec<Diagnostic>,
}

#[pyclass(get_all, frozen, skip_from_py_object)]
#[derive(Clone, Debug, Serialize)]
pub struct Workspace {
    pub kind: String,
    pub path: String,
    pub virtual_root: bool,
    pub members: Vec<String>,
}

#[pyclass(get_all, frozen, skip_from_py_object)]
#[derive(Clone, Debug, Serialize)]
pub struct FileEntry {
    pub path: String,
    pub size: u64,
    pub blob_sha: Option<String>,
}

#[pymethods]
impl FileEntry {
    #[new]
    #[pyo3(signature = (path, size, blob_sha=None))]
    fn new(path: String, size: u64, blob_sha: Option<String>) -> Self {
        Self {
            path,
            size,
            blob_sha,
        }
    }

    fn __repr__(&self) -> String {
        format!("FileEntry(path={:?}, size={})", self.path, self.size)
    }
}

#[pyclass(get_all, frozen, skip_from_py_object)]
#[derive(Clone, Debug, Serialize)]
pub struct WantFile {
    pub path: String,
    pub reason: String,
    pub priority: u32,
    pub max_bytes: u64,
    pub blob_sha: Option<String>,
}

#[pyclass(get_all, frozen, skip_from_py_object)]
#[derive(Clone, Debug, Serialize)]
pub struct ScanResult {
    pub schema_version: u32,
    pub root: String,
    pub upload_root: String,
    pub scan_origin: String,
    pub status: String,
    pub completeness: String,
    pub want_files: Vec<WantFile>,
    pub workspace: Option<Workspace>,
    pub applications: Vec<Application>,
    pub diagnostics: Vec<Diagnostic>,
}

#[pymethods]
impl ScanResult {
    /// Canonical JSON: UTF-8, struct-declaration key order, compact.
    fn to_json(&self) -> String {
        serde_json::to_string(self).expect("model serialization is infallible")
    }

    fn __repr__(&self) -> String {
        format!(
            "ScanResult(applications={}, diagnostics={})",
            self.applications.len(),
            self.diagnostics.len()
        )
    }
}

#[pymethods]
impl Diagnostic {
    fn __repr__(&self) -> String {
        format!(
            "Diagnostic({} [{}] {:?}{})",
            self.code,
            self.severity,
            self.message,
            self.path
                .as_ref()
                .map(|p| format!(" at {p}"))
                .unwrap_or_default()
        )
    }
}
