use std::fs;
use std::path::Path;

use anyhow::{bail, Context};
use serde_json::Value;

use ctx_core::CtxPaths;

use crate::app;
use crate::doctor::hooks_contain_ctx;
use crate::setup::{read_json_object, write_json_atomic};

/// Remove harness hooks, stop the dashboard, and optionally archive ~/.ctx.
pub fn run(purge: bool, yes: bool) -> anyhow::Result<()> {
    let home = dirs::home_dir().context("home directory")?;
    strip_claude(&home)?;
    strip_cursor(&home)?;
    strip_windsurf(&home)?;
    if let Err(err) = app::run(8741, false, false, true) {
        eprintln!("dashboard service: {err}");
    }

    let paths = CtxPaths::default_home()?;
    let root = paths.root().to_path_buf();
    if !root.exists() {
        println!("CTX hooks removed. No store at {}.", root.display());
        return Ok(());
    }

    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let backup = root
        .parent()
        .unwrap_or(root.as_path())
        .join(format!(".ctx.backup.{ts}"));

    if !yes && !purge {
        println!("CTX hooks removed.");
        println!("Store is still at {}", root.display());
        println!("Archive it:  ctx uninstall --purge --yes");
        return Ok(());
    }
    if purge {
        if !yes {
            bail!("refusing to delete {} without --yes", root.display());
        }
        if backup.exists() {
            bail!("backup already exists: {}", backup.display());
        }
        fs::rename(&root, &backup)
            .with_context(|| format!("rename {} -> {}", root.display(), backup.display()))?;
        println!("archived {} → {}", root.display(), backup.display());
        println!("hooks removed. Delete the archive when you are sure.");
    } else {
        println!("CTX hooks removed. Store kept at {}", root.display());
    }
    Ok(())
}

fn strip_claude(home: &Path) -> anyhow::Result<()> {
    let path = home.join(".claude").join("settings.json");
    if !path.exists() {
        println!("  ·  Claude Code  no settings.json");
        return Ok(());
    }
    let mut settings = read_json_object(&path)?;
    let changed = strip_hooks_object(&mut settings, true)?;
    if changed {
        write_json_atomic(&path, &settings)?;
        println!("  ✓  Claude Code  hooks + mcp removed");
    } else {
        println!("  ·  Claude Code  no CTX hooks");
    }
    Ok(())
}

fn strip_cursor(home: &Path) -> anyhow::Result<()> {
    let path = home.join(".cursor").join("hooks.json");
    if !path.exists() {
        println!("  ·  Cursor  no hooks.json");
        return Ok(());
    }
    let mut hooks = read_json_object(&path)?;
    let changed = strip_hooks_object(&mut hooks, false)?;
    if changed {
        write_json_atomic(&path, &hooks)?;
        println!("  ✓  Cursor  hooks removed");
    } else {
        println!("  ·  Cursor  no CTX hooks");
    }
    strip_mcp_file(&home.join(".cursor").join("mcp.json"), "Cursor")?;
    Ok(())
}

fn strip_mcp_file(path: &Path, label: &str) -> anyhow::Result<()> {
    if !path.exists() {
        println!("  ·  {label}  no mcp config");
        return Ok(());
    }
    let mut root = read_json_object(path)?;
    let changed = if let Some(servers) = root.get_mut("mcpServers").and_then(|v| v.as_object_mut())
    {
        servers.remove("ctx").is_some()
    } else {
        false
    };
    if changed {
        write_json_atomic(path, &root)?;
        println!("  ✓  {label}  mcp removed");
    } else {
        println!("  ·  {label}  no CTX mcp");
    }
    Ok(())
}

fn strip_windsurf(home: &Path) -> anyhow::Result<()> {
    strip_mcp_file(
        &home
            .join(".codeium")
            .join("windsurf")
            .join("mcp_config.json"),
        "Windsurf",
    )
}

fn strip_hooks_object(root: &mut Value, strip_mcp: bool) -> anyhow::Result<bool> {
    let mut changed = false;
    if let Some(hooks) = root.get_mut("hooks").and_then(|v| v.as_object_mut()) {
        let keys: Vec<String> = hooks.keys().cloned().collect();
        for k in keys {
            let Some(arr) = hooks.get_mut(&k).and_then(|v| v.as_array_mut()) else {
                continue;
            };
            let before = arr.len();
            arr.retain(|item| !hooks_contain_ctx(Some(item)));
            if arr.len() != before {
                changed = true;
            }
            if arr.is_empty() {
                hooks.remove(&k);
                changed = true;
            }
        }
    }
    if strip_mcp {
        if let Some(servers) = root.get_mut("mcpServers").and_then(|v| v.as_object_mut()) {
            if servers.remove("ctx").is_some() {
                changed = true;
            }
        }
    }
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn strip_removes_ctx_mcp() {
        let mut v = json!({
            "mcpServers": {"ctx": {"command": "/opt/ctx"}, "other": {}},
            "hooks": {
                "PostToolUse": [{"hooks": [{"type": "command", "command": "ctx hook"}]}]
            }
        });
        assert!(strip_hooks_object(&mut v, true).unwrap());
        assert!(v["mcpServers"].get("ctx").is_none());
        assert!(v["mcpServers"].get("other").is_some());
    }
}
