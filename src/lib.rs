mod diag;
mod entrypoint;
mod fileset;
mod manifest;
mod model;
mod node;
mod norm;
mod scan;
mod workspace;

use std::path::PathBuf;

use pyo3::prelude::*;

use crate::model::ScanResult;

/// Statically analyze a directory: find applications, technologies,
/// entrypoints, build facts, and problems without executing user code.
#[pyfunction]
#[pyo3(name = "scan", signature = (root, *, application_dir=None, entrypoint=None, max_files=None, follow_symlinks=false, extra_ignore_files=None))]
fn scan_py(
    py: Python<'_>,
    root: PathBuf,
    application_dir: Option<String>,
    entrypoint: Option<String>,
    max_files: Option<u64>,
    follow_symlinks: bool,
    extra_ignore_files: Option<Vec<String>>,
) -> PyResult<ScanResult> {
    let opts = scan::ScanOptions {
        application_dir,
        entrypoint,
        max_files,
        follow_symlinks,
        extra_ignore_files: extra_ignore_files.unwrap_or_default(),
    };
    // Release the GIL: scans are pure Rust and may run in parallel threads.
    Ok(py.detach(|| scan::scan(&root, &opts)))
}

#[pymodule(gil_used = false, name = "_kenbun")]
fn kenbun(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(scan_py, m)?)?;
    m.add_class::<model::ScanResult>()?;
    m.add_class::<model::Workspace>()?;
    m.add_class::<model::Application>()?;
    m.add_class::<model::Technology>()?;
    m.add_class::<model::BuildScript>()?;
    m.add_class::<model::Entrypoint>()?;
    m.add_class::<model::EnvVar>()?;
    m.add_class::<model::DependencySet>()?;
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
    Ok(())
}
