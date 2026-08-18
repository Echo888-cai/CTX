use ctx_core::Runtime;

pub fn run(query: &str, limit: usize) -> anyhow::Result<()> {
    let rt = Runtime::open_default()?;
    let out = rt.search(query, limit)?;
    print!("{out}");
    if !out.ends_with('\n') {
        println!();
    }
    Ok(())
}
