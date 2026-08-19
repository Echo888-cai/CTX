use anyhow::Context;
use ctx_core::{CtxPaths, Runtime};

pub fn create(note: Option<&str>) -> anyhow::Result<()> {
    let rt = Runtime::open_default().context("open CTX store")?;
    let snap = rt.store.create_snapshot(note)?;
    println!("snapshot  {}", snap.id);
    println!("  {}", snap.path.display());
    if !snap.note.is_empty() {
        println!("  {}", snap.note);
    }
    Ok(())
}

pub fn list() -> anyhow::Result<()> {
    let rt = Runtime::open_default().context("open CTX store")?;
    let snaps = rt.store.list_snapshots()?;
    if snaps.is_empty() {
        println!("No snapshots. Create one with: ctx snapshot create");
        return Ok(());
    }
    println!("Snapshots");
    println!();
    for s in snaps {
        let note = if s.note.is_empty() {
            String::new()
        } else {
            format!("  {}", s.note)
        };
        println!("  {}  {}{note}", s.id, s.created_at);
    }
    println!();
    println!("Restore: ctx snapshot restore <id>");
    Ok(())
}

pub fn restore(id: &str) -> anyhow::Result<()> {
    let paths = CtxPaths::default_home()?;
    let store = ctx_core::Store::open(paths)?;
    store.restore_snapshot(id)?;
    println!("restored snapshot {id}");
    println!("Raw ctx:// pages are unchanged. Run ctx doctor if hooks look off.");
    Ok(())
}

pub fn versions() -> anyhow::Result<()> {
    println!("ctx  {}", env!("CARGO_PKG_VERSION"));
    let paths = CtxPaths::default_home().ok();
    if let Some(paths) = paths {
        let dir = paths.versions_dir();
        if dir.is_dir() {
            let mut bins: Vec<_> = std::fs::read_dir(&dir)
                .into_iter()
                .flatten()
                .flatten()
                .filter(|e| e.path().is_dir())
                .collect();
            bins.sort_by_key(|e| e.file_name());
            if !bins.is_empty() {
                println!();
                println!("Installed copies");
                for e in bins {
                    println!("  {}", e.file_name().to_string_lossy());
                }
            }
        }
    }
    println!();
    println!("Pin this binary:   ctx version pin");
    println!("Switch binary:     ctx version use <version>");
    println!("Data rollback:     ctx snapshot restore <id>");
    Ok(())
}

pub fn bin_name() -> &'static str {
    if cfg!(windows) {
        "ctx.exe"
    } else {
        "ctx"
    }
}

/// Copy the running binary into ~/.ctx/versions/<pkg-version>/.
pub fn pin() -> anyhow::Result<()> {
    let paths = CtxPaths::default_home()?;
    let ver = env!("CARGO_PKG_VERSION");
    let dest_dir = paths.versions_dir().join(ver);
    std::fs::create_dir_all(&dest_dir).with_context(|| format!("create {}", dest_dir.display()))?;
    let exe = std::env::current_exe().context("current exe")?;
    let dest = dest_dir.join(bin_name());
    std::fs::copy(&exe, &dest)
        .with_context(|| format!("copy {} -> {}", exe.display(), dest.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&dest)?.permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&dest, perms)?;
    }
    println!("pinned {ver} → {}", dest.display());
    Ok(())
}

pub fn use_version(id: &str) -> anyhow::Result<()> {
    let paths = CtxPaths::default_home()?;
    let src = paths.versions_dir().join(id).join(bin_name());
    if !src.is_file() {
        anyhow::bail!(
            "no pinned binary at {}
Pin first: ctx version pin",
            src.display()
        );
    }
    let exe = std::env::current_exe().context("current exe")?;
    let tmp = exe.with_file_name(format!("{}.new", bin_name()));
    std::fs::copy(&src, &tmp)
        .with_context(|| format!("copy {} -> {}", src.display(), tmp.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&tmp)?.permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&tmp, perms)?;
    }
    match std::fs::rename(&tmp, &exe) {
        Ok(()) => {}
        Err(_) => {
            std::fs::copy(&src, &exe).with_context(|| format!("replace {}", exe.display()))?;
            let _ = std::fs::remove_file(&tmp);
        }
    }
    println!("now using {id}");
    println!("restart shells and IDE hooks so they pick up the binary");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bin_name_is_ctx() {
        if cfg!(windows) {
            assert_eq!(bin_name(), "ctx.exe");
        } else {
            assert_eq!(bin_name(), "ctx");
        }
    }
}
