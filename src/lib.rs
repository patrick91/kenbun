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

use pyo3::exceptions::{PyTypeError, PyValueError};
use pyo3::prelude::*;

use crate::model::ScanResult;

#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn getpid() -> i32;
}

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
#[pyo3(signature = (files, contents=None, *, inventory_complete=true, hints=None, max_file_bytes=2_097_152))]
fn analyze(
    py: Python<'_>,
    files: &Bound<'_, PyAny>,
    contents: Option<&Bound<'_, PyAny>>,
    inventory_complete: bool,
    hints: Option<BTreeMap<String, Vec<String>>>,
    max_file_bytes: u64,
) -> PyResult<ScanResult> {
    if max_file_bytes == 0 {
        return Err(PyValueError::new_err(
            "max_file_bytes must be a positive integer",
        ));
    }
    let paths = files
        .try_iter()?
        .enumerate()
        .map(|(index, entry)| {
            entry?
                .extract::<String>()
                .map_err(|_| PyTypeError::new_err(format!("files[{index}] must be a string")))
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
    let fs = fileset::virtual_files(paths, extracted_contents, script_patterns, max_file_bytes)
        .map_err(PyValueError::new_err)?;
    Ok(py.detach(|| scan::analyze(&fs, inventory_complete)))
}

#[pymodule(gil_used = false, name = "_kenbun")]
fn kenbun(m: &Bound<'_, PyModule>) -> PyResult<()> {
    // macOS 27 requires the Mach-O string table to be 8-byte aligned, while
    // Apple's linker omits padding after an odd-length indirect symbol table.
    // Materializing one non-lazy system symbol keeps the table aligned.
    #[cfg(target_os = "macos")]
    std::hint::black_box(getpid as unsafe extern "C" fn() -> i32);

    m.add_function(wrap_pyfunction!(scan_py, m)?)?;
    m.add_function(wrap_pyfunction!(analyze, m)?)?;
    m.add_class::<model::FileRequest>()?;
    m.add_class::<model::ScanResult>()?;
    m.add_class::<model::Workspace>()?;
    m.add_class::<model::Application>()?;
    m.add_class::<model::Technology>()?;
    m.add_class::<model::BuildScript>()?;
    m.add_class::<model::Entrypoint>()?;
    m.add_class::<model::EnvVar>()?;
    m.add_class::<model::DependencySet>()?;
    m.add_class::<model::DeclaredDep>()?;
    m.add_class::<model::ManifestRef>()?;
    m.add_class::<model::SourceRef>()?;
    m.add_class::<model::PythonInfo>()?;
    m.add_class::<model::NodeInfo>()?;
    m.add_class::<model::VersionPin>()?;
    m.add_class::<model::Diagnostic>()?;
    m.add_class::<model::Evidence>()?;
    m.add_class::<model::Span>()?;
    Ok(())
}
