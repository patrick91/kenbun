mod diag;
mod entrypoint;
mod fileset;
mod manifest;
mod model;
mod norm;
mod scan;
mod workspace;

use std::path::PathBuf;

use pyo3::prelude::*;

use crate::model::ScanResult;

/// Statically analyze a directory: find Python projects, deployable web
/// apps, entrypoints, and problems — without importing user code.
#[pyfunction]
#[pyo3(name = "scan", signature = (root, *, target_dir=None, entrypoint=None, max_files=None, follow_symlinks=false, extra_ignore_files=None))]
fn scan_py(
    py: Python<'_>,
    root: PathBuf,
    target_dir: Option<String>,
    entrypoint: Option<String>,
    max_files: Option<u64>,
    follow_symlinks: bool,
    extra_ignore_files: Option<Vec<String>>,
) -> PyResult<ScanResult> {
    let opts = scan::ScanOptions {
        target_dir,
        entrypoint,
        max_files,
        follow_symlinks,
        extra_ignore_files: extra_ignore_files.unwrap_or_default(),
    };
    // Release the GIL: scans are pure Rust and may run in parallel threads.
    Ok(py.detach(|| scan::scan(&root, &opts)))
}

#[pyfunction]
fn hello() -> &'static str {
    "Hello, world!"
}

#[pymodule(gil_used = false, name = "_kenbun")]
fn kenbun(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(hello, m)?)?;
    m.add_function(wrap_pyfunction!(scan_py, m)?)?;
    m.add_class::<model::ScanResult>()?;
    m.add_class::<model::Workspace>()?;
    m.add_class::<model::Project>()?;
    m.add_class::<model::DeployTarget>()?;
    m.add_class::<model::Entrypoint>()?;
    m.add_class::<model::EnvVar>()?;
    m.add_class::<model::Dependencies>()?;
    m.add_class::<model::DeclaredDep>()?;
    m.add_class::<model::ResolvedDep>()?;
    m.add_class::<model::ManifestRef>()?;
    m.add_class::<model::LockfileRef>()?;
    m.add_class::<model::SourceRef>()?;
    m.add_class::<model::PythonInfo>()?;
    m.add_class::<model::VersionPin>()?;
    m.add_class::<model::Diagnostic>()?;
    m.add_class::<model::Evidence>()?;
    m.add_class::<model::Span>()?;
    m.add_class::<model::WantFile>()?;
    m.add_class::<model::Classification>()?;
    m.add_class::<model::ClassificationPrimary>()?;
    m.add_class::<model::InputInfo>()?;
    Ok(())
}
