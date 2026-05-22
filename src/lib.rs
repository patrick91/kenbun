use pyo3::prelude::*;

#[pyfunction]
fn hello() -> &'static str {
    "Hello, world!"
}

#[pymodule(gil_used = false)]
fn kenbun(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(hello, m)?)?;
    Ok(())
}
