use std::fs;
use std::path::{Path, PathBuf};

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
    strip_vscode(&home)?;
    strip_continue(&home)?;
    strip_jetbrains(&home)?;
    strip_codex(&home)?;
    strip_claude_desktop(&home)?;
    if let Err(err) = app::run(8741, false, false, true, false) {
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
    let mut changed = false;
    if let Some(servers) = root.get_mut("mcpServers").and_then(|v| v.as_object_mut()) {
        changed |= servers.remove("ctx").is_some();
    }
    if let Some(servers) = root.get_mut("servers").and_then(|v| v.as_object_mut()) {
        changed |= servers.remove("ctx").is_some();
    }
    if changed {
        write_json_atomic(path, &root)?;
        println!("  ✓  {label}  mcp removed");
    } else {
        println!("  ·  {label}  no CTX mcp");
    }
    Ok(())
}

fn strip_claude_desktop(home: &Path) -> anyhow::Result<()> {
    let path = crate::setup::claude_desktop_config_path(home);
    strip_mcp_file(&path, "Claude Desktop")
}

/// Remove CTX from a single harness. Used by the dashboard settings drawer.
pub fn strip_target(target: &str) -> anyhow::Result<()> {
    let home = dirs::home_dir().context("home directory")?;
    match target {
        "claude" | "claude-code" => strip_claude(&home),
        "claude-desktop" => strip_claude_desktop(&home),
        "cursor" => strip_cursor(&home),
        "windsurf" => strip_windsurf(&home),
        "vscode" | "code" => strip_vscode(&home),
        "continue" => strip_continue(&home),
        "jetbrains" | "idea" => strip_jetbrains(&home),
        "codex" => strip_codex(&home),
        "copilot" => strip_vscode(&home),
        "aider" => Ok(()),
        other => bail!("unknown target {other}"),
    }
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

fn strip_vscode(home: &Path) -> anyhow::Result<()> {
    for path in [
        home.join("Library")
            .join("Application Support")
            .join("Code")
            .join("User")
            .join("mcp.json"),
        home.join(".config")
            .join("Code")
            .join("User")
            .join("mcp.json"),
        home.join("AppData")
            .join("Roaming")
            .join("Code")
            .join("User")
            .join("mcp.json"),
        PathBuf::from(".vscode").join("mcp.json"),
    ] {
        if path.exists() {
            strip_mcp_file(&path, "VS Code")?;
        }
    }
    Ok(())
}

fn strip_continue(home: &Path) -> anyhow::Result<()> {
    let path = home.join(".continue").join("mcpServers").join("ctx.yaml");
    if path.exists() {
        fs::remove_file(&path)?;
        println!("  ✓  Continue  ctx.yaml removed");
    } else {
        println!("  ·  Continue  no ctx.yaml");
    }
    Ok(())
}

fn strip_jetbrains(home: &Path) -> anyhow::Result<()> {
    strip_mcp_file(
        &home
            .join("Library")
            .join("Application Support")
            .join("JetBrains")
            .join("mcp.json"),
        "JetBrains",
    )?;
    strip_mcp_file(
        &home.join(".config").join("JetBrains").join("mcp.json"),
        "JetBrains",
    )?;
    strip_mcp_file(&PathBuf::from(".idea").join("mcp.json"), "JetBrains")?;
    Ok(())
}

fn strip_codex(home: &Path) -> anyhow::Result<()> {
    let path = home.join(".codex").join("config.toml");
    if !path.exists() {
        println!("  ·  Codex  no config.toml");
        return Ok(());
    }
    let text = fs::read_to_string(&path)?;
    if !text.contains("[mcp_servers.ctx]") {
        println!("  ·  Codex  no CTX mcp");
        return Ok(());
    }
    let mut out = String::new();
    let mut skip = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed == "[mcp_servers.ctx]" {
            skip = true;
            continue;
        }
        if skip {
            if trimmed.starts_with('[') {
                skip = false;
            } else {
                continue;
            }
        }
        if !skip {
            out.push_str(line);
            out.push('\n');
        }
    }
    fs::write(&path, out)?;
    println!("  ✓  Codex  mcp removed");
    Ok(())
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
