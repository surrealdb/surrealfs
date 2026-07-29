fn main() -> pyo3_stub_gen::Result<()> {
    let stub = surrealfs::python::stub_info()?;
    stub.generate()?;
    Ok(())
}
