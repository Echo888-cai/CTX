use ctx_core::{format_why, Runtime, Snapshot};

pub fn run() -> anyhow::Result<()> {
    let rt = Runtime::open_default()?;
    let snap = Snapshot::capture(&rt.store)?;
    println!(
        "{}",
        format_why(&snap.reasons_today, snap.today.avoided, snap.pages)
    );
    Ok(())
}
