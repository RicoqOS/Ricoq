fn main() -> Result<(), std::env::VarError> {
    let directory = std::env::var("CARGO_MANIFEST_DIR")?;
    let script = format!("{directory}/tests/root-task/test-page.ld");
    println!("cargo::rerun-if-changed={script}");
    println!("cargo::rustc-link-arg-tests=-T{script}");
    Ok(())
}
