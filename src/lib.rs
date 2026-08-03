mod diag;
mod ecosystems;
mod fileset;
mod model;
mod scan;

use std::collections::BTreeMap;
use std::path::PathBuf;

use pyo3::exceptions::{PyKeyError, PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBool, PyString};

use crate::model::ScanResult;

#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn getpid() -> i32;
}

/// Statically analyze a directory: find applications, technologies,
/// entrypoints, build facts, and problems without executing user code.
#[pyfunction]
#[pyo3(name = "scan", signature = (root, *, ecosystems=None, application_dir=None, entrypoint=None, max_files=None, follow_symlinks=false, extra_ignore_files=None, max_depth=None))]
#[expect(clippy::too_many_arguments, reason = "mirrors the Python API")]
fn scan_py(
    py: Python<'_>,
    root: PathBuf,
    ecosystems: Option<&Bound<'_, PyAny>>,
    application_dir: Option<String>,
    entrypoint: Option<String>,
    max_files: Option<u64>,
    follow_symlinks: bool,
    extra_ignore_files: Option<Vec<String>>,
    max_depth: Option<&Bound<'_, PyAny>>,
) -> PyResult<ScanResult> {
    let max_depth = extract_max_depth(max_depth)?;
    let ecosystems = extract_ecosystems(ecosystems)?;
    if !ecosystems.python && entrypoint.is_some() {
        return Err(PyValueError::new_err(
            "entrypoint requires the 'python' ecosystem",
        ));
    }
    let opts = scan::ScanOptions {
        application_dir,
        entrypoint,
        ecosystems,
        max_files,
        follow_symlinks,
        extra_ignore_files: extra_ignore_files.unwrap_or_default(),
        max_depth,
    };
    // Release the GIL: scans are pure Rust and may run in parallel threads.
    Ok(py.detach(|| scan::scan(&root, &opts)))
}

/// Analyze a caller-provided repository inventory without filesystem or
/// network access. Missing contents are returned as ordered file requests.
#[pyfunction]
#[pyo3(signature = (files, contents=None, *, ecosystems=None, inventory_complete=true, hints=None, max_files=None, max_file_bytes=2_097_152, max_depth=None))]
#[expect(clippy::too_many_arguments, reason = "mirrors the Python API")]
fn analyze(
    py: Python<'_>,
    files: &Bound<'_, PyAny>,
    contents: Option<&Bound<'_, PyAny>>,
    ecosystems: Option<&Bound<'_, PyAny>>,
    inventory_complete: bool,
    hints: Option<BTreeMap<String, Vec<String>>>,
    max_files: Option<u64>,
    max_file_bytes: u64,
    max_depth: Option<&Bound<'_, PyAny>>,
) -> PyResult<ScanResult> {
    if max_file_bytes == 0 {
        return Err(PyValueError::new_err(
            "max_file_bytes must be a positive integer",
        ));
    }
    let max_depth = extract_max_depth(max_depth)?;
    let entries = files
        .try_iter()?
        .enumerate()
        .map(|(index, entry)| {
            let entry = entry?;
            let path = entry
                .get_item("path")
                .and_then(|path| path.extract::<String>())
                .map_err(|_| {
                    PyTypeError::new_err(format!("files[{index}].path must be a string"))
                })?;
            let size = entry.get_item("size").map_err(|_| {
                PyTypeError::new_err(format!(
                    "files[{index}].size must be a non-negative integer or None"
                ))
            })?;
            let size = if size.is_none() {
                0
            } else if size.is_instance_of::<PyBool>() {
                return Err(PyTypeError::new_err(format!(
                    "files[{index}].size must be a non-negative integer or None"
                )));
            } else {
                size.extract::<u64>().map_err(|_| {
                    PyTypeError::new_err(format!(
                        "files[{index}].size must be a non-negative integer or None"
                    ))
                })?
            };
            let is_symlink = match entry.get_item("is_symlink") {
                Ok(value) if value.is_instance_of::<PyBool>() => {
                    value.extract::<bool>().map_err(|_| {
                        PyTypeError::new_err(format!("files[{index}].is_symlink must be a bool"))
                    })?
                }
                Ok(_) => {
                    return Err(PyTypeError::new_err(format!(
                        "files[{index}].is_symlink must be a bool"
                    )));
                }
                Err(error) if error.is_instance_of::<PyKeyError>(py) => false,
                Err(error) => return Err(error),
            };
            Ok((path, size, is_symlink))
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
    let fs = fileset::virtual_files(
        entries,
        extracted_contents,
        script_patterns,
        max_files,
        max_file_bytes,
        max_depth,
    )
    .map_err(PyValueError::new_err)?;
    let ecosystems = extract_ecosystems(ecosystems)?;
    Ok(py.detach(|| scan::analyze(&fs, inventory_complete, ecosystems)))
}

fn extract_max_depth(value: Option<&Bound<'_, PyAny>>) -> PyResult<Option<u64>> {
    let Some(value) = value else {
        return Ok(None);
    };
    if value.is_instance_of::<PyBool>() {
        return Err(PyValueError::new_err(
            "max_depth must be a non-negative integer or None",
        ));
    }
    value
        .extract::<u64>()
        .map(Some)
        .map_err(|_| PyValueError::new_err("max_depth must be a non-negative integer or None"))
}

/// Validate Python's flexible iterable input at the public boundary.
fn extract_ecosystems(value: Option<&Bound<'_, PyAny>>) -> PyResult<scan::EcosystemSelection> {
    let Some(value) = value else {
        return Ok(scan::EcosystemSelection::default());
    };
    if value.is_instance_of::<PyString>() {
        return Err(PyTypeError::new_err(
            "ecosystems must be an iterable of ecosystem names, not a string",
        ));
    }

    let items = value
        .try_iter()
        .map_err(|_| PyTypeError::new_err("ecosystems must be an iterable of ecosystem names"))?;
    let mut selection = scan::EcosystemSelection {
        python: false,
        node: false,
    };
    let mut selected = false;
    for item in items {
        let name = item?
            .extract::<String>()
            .map_err(|_| PyTypeError::new_err("ecosystems must contain only strings"))?;
        selected = true;
        match name.as_str() {
            "python" => selection.python = true,
            "node" => selection.node = true,
            _ => {
                return Err(PyValueError::new_err(format!(
                    "unknown ecosystem '{name}'; expected 'python' or 'node'"
                )));
            }
        }
    }
    if !selected {
        return Err(PyValueError::new_err(
            "ecosystems must contain at least one of: 'python', 'node'",
        ));
    }
    Ok(selection)
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
