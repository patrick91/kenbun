//! Output model — spec §4. Field order here IS the JSON key order (§14).

use pyo3::prelude::*;
use serde::Serialize;

pub const SCHEMA_VERSION: u32 = 0;

#[pyclass(get_all, frozen)]
#[derive(Clone, Debug, Serialize)]
pub struct Span {
    pub start_line: u32,
    pub start_col: u32,
    pub end_line: u32,
    pub end_col: u32,
}

#[pyclass(get_all, frozen)]
#[derive(Clone, Debug, Serialize)]
pub struct Diagnostic {
    pub code: String,
    pub severity: String,
    pub message: String,
    pub path: Option<String>,
    pub span: Option<Span>,
}

#[pyclass(get_all, frozen)]
#[derive(Clone, Debug, Serialize)]
pub struct Evidence {
    pub kind: String,
    pub path: String,
    pub span: Option<Span>,
    pub detail: String,
}

#[pyclass(get_all, frozen)]
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

#[pyclass(get_all, frozen)]
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

#[pyclass(get_all, frozen)]
#[derive(Clone, Debug, Serialize)]
pub struct SourceRef {
    pub path: String,
    pub span: Option<Span>,
}

#[pyclass(get_all, frozen)]
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

#[pyclass(get_all, frozen)]
#[derive(Clone, Debug, Serialize)]
pub struct ResolvedDep {
    pub name: String,
    pub version: String,
    pub source: String,
    pub marker: Option<String>,
}

#[pyclass(get_all, frozen)]
#[derive(Clone, Debug, Serialize)]
pub struct ManifestRef {
    pub path: String,
    pub kind: String,
}

#[pyclass(get_all, frozen)]
#[derive(Clone, Debug, Serialize)]
pub struct LockfileRef {
    pub path: String,
    pub kind: String,
    pub parsed: bool,
}

#[pyclass(get_all, frozen)]
#[derive(Clone, Debug, Serialize)]
pub struct Dependencies {
    pub package_manager: String,
    pub manifests: Vec<ManifestRef>,
    pub lockfiles: Vec<LockfileRef>,
    pub declared: Vec<DeclaredDep>,
    pub resolved: Vec<ResolvedDep>,
}

#[pyclass(get_all, frozen)]
#[derive(Clone, Debug, Serialize)]
pub struct VersionPin {
    pub source: String,
    pub value: String,
}

#[pyclass(get_all, frozen)]
#[derive(Clone, Debug, Serialize)]
pub struct PythonInfo {
    pub requires_python: Option<String>,
    pub version_pins: Vec<VersionPin>,
}

#[pyclass(get_all, frozen)]
#[derive(Clone, Debug, Serialize)]
pub struct DeployTarget {
    pub framework: String,
    pub form: String,
    pub project_path: String,
    pub entrypoint: Option<Entrypoint>,
    pub confidence: String,
    pub recommended: bool,
    pub env_vars: Vec<EnvVar>,
    pub evidence: Vec<Evidence>,
    pub diagnostics: Vec<Diagnostic>,
}

#[pyclass(get_all, frozen)]
#[derive(Clone, Debug, Serialize)]
pub struct Project {
    pub path: String,
    pub name: Option<String>,
    pub roles: Vec<String>,
    pub frameworks: Vec<String>,
    pub deploy_targets: Vec<DeployTarget>,
    pub dependencies: Option<Dependencies>,
    pub env_vars: Vec<EnvVar>,
    pub python: PythonInfo,
    pub evidence: Vec<Evidence>,
    pub diagnostics: Vec<Diagnostic>,
}

#[pyclass(get_all, frozen)]
#[derive(Clone, Debug, Serialize)]
pub struct Workspace {
    pub kind: String,
    pub path: String,
    pub virtual_root: bool,
    pub members: Vec<String>,
}

#[pyclass(get_all, frozen)]
#[derive(Clone, Debug, Serialize)]
pub struct WantFile {
    pub path: String,
    pub reason: String,
    pub priority: u32,
    pub max_bytes: u64,
    pub blob_sha: Option<String>,
}

#[pyclass(get_all, frozen)]
#[derive(Clone, Debug, Serialize)]
pub struct ClassificationPrimary {
    pub path: String,
    pub evidence: String,
}

#[pyclass(get_all, frozen)]
#[derive(Clone, Debug, Serialize)]
pub struct Classification {
    pub python: String,
    pub uses_fastapi: String,
    pub primary: Option<ClassificationPrimary>,
}

#[pyclass(get_all, frozen)]
#[derive(Clone, Debug, Serialize)]
pub struct InputInfo {
    pub mode: String,
    pub files_seen: u64,
    pub complete: bool,
}

#[pyclass(get_all, frozen)]
#[derive(Clone, Debug, Serialize)]
pub struct ScanResult {
    pub schema_version: u32,
    pub root: String,
    pub upload_root: String,
    pub scan_origin: String,
    pub status: String,
    pub want_files: Vec<WantFile>,
    pub input: InputInfo,
    pub workspace: Option<Workspace>,
    pub projects: Vec<Project>,
    pub deploy_targets: Vec<DeployTarget>,
    pub classification: Classification,
    pub diagnostics: Vec<Diagnostic>,
}

#[pymethods]
impl ScanResult {
    /// Canonical JSON (§14): UTF-8, struct-declaration key order, compact.
    fn to_json(&self) -> String {
        serde_json::to_string(self).expect("model serialization is infallible")
    }

    fn __repr__(&self) -> String {
        format!(
            "ScanResult(projects={}, deploy_targets={}, diagnostics={})",
            self.projects.len(),
            self.deploy_targets.len(),
            self.diagnostics.len()
        )
    }
}

#[pymethods]
impl DeployTarget {
    fn __repr__(&self) -> String {
        format!(
            "DeployTarget(framework={:?}, project_path={:?}, entrypoint={:?}, confidence={:?}, recommended={})",
            self.framework,
            self.project_path,
            self.entrypoint.as_ref().map(|e| e.as_string.clone()),
            self.confidence,
            self.recommended
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
