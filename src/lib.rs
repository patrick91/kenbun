mod diag;
mod entrypoint;
mod fileset;
mod manifest;
mod model;
mod node;
mod norm;
mod runtime;
mod scan;
mod workspace;

use std::collections::BTreeMap;
use std::path::PathBuf;

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

use crate::model::{FileEntry, ScanResult};

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

/// Analyze a caller-provided repository inventory without filesystem or
/// network access. Missing contents are returned as ordered file requests.
#[pyfunction]
#[pyo3(signature = (files, contents=None, *, inventory_complete=true, hints=None))]
fn analyze(
    py: Python<'_>,
    files: &Bound<'_, PyAny>,
    contents: Option<&Bound<'_, PyAny>>,
    inventory_complete: bool,
    hints: Option<BTreeMap<String, Vec<String>>>,
) -> PyResult<ScanResult> {
    let entries = files
        .try_iter()?
        .map(|entry| {
            let entry = entry?;
            Ok(entry.extract::<PyRef<'_, FileEntry>>()?.clone())
        })
        .collect::<PyResult<Vec<_>>>()?;
    let mut extracted_contents = BTreeMap::new();
    if let Some(contents) = contents {
        for item in contents.call_method0("items")?.try_iter()? {
            let (path, content) = item?.extract::<(String, Option<Vec<u8>>)>()?;
            extracted_contents.insert(path, content);
        }
    }
    let mut hints = hints.unwrap_or_default();
    let script_patterns = hints.remove("script_patterns").unwrap_or_default();
    if let Some(key) = hints.keys().next() {
        return Err(PyValueError::new_err(format!(
            "unknown analysis hint: {key}"
        )));
    }
    let fs = fileset::virtual_files(entries, extracted_contents, script_patterns)
        .map_err(PyValueError::new_err)?;
    Ok(py.detach(|| scan::analyze(&fs, inventory_complete)))
}

#[pymodule(gil_used = false, name = "_kenbun")]
fn kenbun(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(scan_py, m)?)?;
    m.add_function(wrap_pyfunction!(analyze, m)?)?;
    m.add_class::<model::FileEntry>()?;
    m.add_class::<model::WantFile>()?;
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
    m.add_class::<model::NodeInfo>()?;
    m.add_class::<model::VersionPin>()?;
    m.add_class::<model::Diagnostic>()?;
    m.add_class::<model::Evidence>()?;
    m.add_class::<model::Span>()?;
    Ok(())
}
