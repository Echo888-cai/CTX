use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context};
use serde_json::{json, Value};

use ctx_core::{single_quote, Config, CtxPaths, Store};

use crate::doctor::{hooks_contain_ctx, is_ctx_hook_command};

pub fn init() -> anyhow::Result<()> {
    let paths = CtxPaths::default_home()?;
    Store::open(paths.clone())?;
    Config::default().save(&paths)?;

    let claude = detect_claude();
    let claude_desktop = detect_claude_desktop();
    let cursor = detect_cursor();
    let windsurf = detect_windsurf();
    let vscode = detect_vscode();
    let cont = detect_continue();
    let jetbrains = detect_jetbrains();
    let aider = detect_aider();
    let codex = detect_codex();

    println!("CTX  {}", paths.root().display());
    println!();
    println!("Detected");
    println!("  {}  Claude Code", mark(claude));
    println!("  {}  Claude Desktop", mark(claude_desktop));
    println!("  {}  Cursor", mark(cursor));
    println!("  {}  Windsurf", mark(windsurf));
    println!("  {}  VS Code / Copilot", mark(vscode));
    println!("  {}  Continue.dev", mark(cont));
    println!("  {}  JetBrains", mark(jetbrains));
    println!("  {}  Aider", mark(aider));
    println!("  {}  Codex CLI", mark(codex));
    println!();

    if claude {
        setup_claude()?;
        println!("  ✓  Claude Code  hooks + mcp");
    }
    if claude_desktop {
        setup_claude_desktop()?;
        println!("  ✓  Claude Desktop  mcp");
    }
    if cursor {
        setup_cursor()?;
        println!("  ✓  Cursor       hooks + mcp");
    }
    if windsurf {
        setup_windsurf()?;
        println!("  ✓  Windsurf     mcp");
    }
    if vscode {
        let _ = setup_vscode();
        let _ = setup_copilot();
        println!("  ✓  VS Code      mcp");
    }
    if cont {
        setup_continue()?;
        println!("  ✓  Continue     mcp");
    }
    if jetbrains {
        setup_jetbrains()?;
        println!("  ✓  JetBrains    mcp");
    }
    if aider {
        setup_aider()?;
        println!("  ✓  Aider        wrapper");
    }
    if codex {
        setup_codex()?;
        println!("  ✓  Codex        mcp");
    }
    if !claude && !claude_desktop && !cursor && !windsurf && !vscode && !cont && !jetbrains && !aider && !codex {
        println!("  ·  none — later: ctx setup claude, cursor, windsurf, vscode, continue, jetbrains, aider, or codex");
    }

    let _ = crate::snapshot::pin();
    println!();
    println!("Next: ctx demo · ctx doctor");
    Ok(())
}

pub fn setup(target: &str) -> anyhow::Result<()> {
    let paths = CtxPaths::default_home()?;
    Store::open(paths.clone())?;
    match target {
        "claude" | "claude-code" => {
            setup_claude()?;
            println!("Claude Code hooks installed. ctx doctor to verify.");
            Ok(())
        }
        "claude-desktop" => {
            setup_claude_desktop()?;
            println!("Claude Desktop MCP installed. Restart Claude to pick up ctx.");
            Ok(())
        }
        "cursor" => {
            setup_cursor()?;
            println!("Cursor hooks installed. ctx doctor to verify.");
            Ok(())
        }
        "windsurf" => {
            setup_windsurf()?;
            println!("Windsurf MCP installed. Refresh Cascade MCP settings.");
            Ok(())
        }
        "vscode" | "code" => {
            setup_vscode()?;
            println!("VS Code MCP installed. Reload the window to pick up ctx.");
            Ok(())
        }
        "continue" => {
            setup_continue()?;
            println!("Continue.dev MCP installed. Reload Continue to pick up ctx.");
            Ok(())
        }
        "jetbrains" | "idea" | "goland" | "pycharm" => {
            setup_jetbrains()?;
            println!("JetBrains MCP installed. Restart the IDE AI chat.");
            Ok(())
        }
        "aider" => {
            setup_aider()?;
            Ok(())
        }
        "codex" => {
            setup_codex()?;
            println!("Codex CLI MCP installed. Restart Codex.");
            Ok(())
        }
        "copilot" => {
            setup_copilot()?;
            println!("GitHub Copilot MCP installed (VS Code user mcp.json).");
            Ok(())
        }
        "all" => {
            setup_claude()?;
            let _ = setup_claude_desktop();
            setup_cursor()?;
            setup_windsurf()?;
            let _ = setup_vscode();
            let _ = setup_continue();
            let _ = setup_jetbrains();
            let _ = setup_aider();
            let _ = setup_codex();
            let _ = setup_copilot();
            println!("Harness adapters installed. ctx doctor to verify.");
            Ok(())
        }
        "wizard" => wizard(),
        other => {
            anyhow::bail!(
                "unknown target {other:?} (use claude, claude-desktop, cursor, windsurf, vscode, continue, jetbrains, aider, copilot, codex, all, or wizard)"
            )
        }
    }
}

pub fn wizard() -> anyhow::Result<()> {
    use std::io::{self, IsTerminal, Write};

    let paths = CtxPaths::default_home()?;
    Store::open(paths.clone())?;
    let mut cfg = Config::load(&paths);

    println!("CTX setup wizard");
    println!();
    let claude = detect_claude();
    let cursor = detect_cursor();
    let windsurf = detect_windsurf();
    let vscode = detect_vscode();
    let cont = detect_continue();
    let jetbrains = detect_jetbrains();
    let aider = detect_aider();
    let codex = detect_codex();
    println!("[1/5] Detected");
    println!("  {}  Claude Code", mark(claude));
    println!("  {}  Cursor", mark(cursor));
    println!("  {}  Windsurf", mark(windsurf));
    println!("  {}  VS Code / Copilot", mark(vscode));
    println!("  {}  Continue.dev", mark(cont));
    println!("  {}  JetBrains", mark(jetbrains));
    println!("  {}  Aider", mark(aider));
    println!("  {}  Codex CLI", mark(codex));

    let interactive = io::stdin().is_terminal();
    let strategy = if interactive {
        println!();
        println!("[2/5] Context budget");
        println!("  1. extreme        — fewer tokens, may drop detail");
        println!("  2. balanced       — recommended");
        println!("  3. conservative   — keep more of the log");
        print!("choice [2]: ");
        let _ = io::stdout().flush();
        let mut line = String::new();
        io::stdin().read_line(&mut line)?;
        match line.trim() {
            "1" | "extreme" => "extreme",
            "3" | "conservative" => "conservative",
            _ => "balanced",
        }
    } else {
        println!();
        println!("[2/5] Context budget: balanced (non-interactive)");
        "balanced"
    };
    cfg.budget_strategy = strategy.into();

    if interactive {
        println!();
        println!("[3/5] Auto-start dashboard at login? [y/N]");
        let mut line = String::new();
        io::stdin().read_line(&mut line)?;
        cfg.dashboard_autostart = matches!(line.trim(), "y" | "Y" | "yes");
        println!("[4/5] Auto snapshot every 24h? [y/N]");
        line.clear();
        io::stdin().read_line(&mut line)?;
        cfg.auto_snapshot = matches!(line.trim(), "y" | "Y" | "yes");
    } else {
        println!();
        println!("[3/5] Dashboard autostart: off (non-interactive)");
        println!("[4/5] Auto snapshot: off (non-interactive)");
    }
    cfg.save(&paths)?;
    if cfg.dashboard_autostart {
        let _ = crate::app::run(8741, false, true, false, false);
    }

    println!();
    println!("[5/5] Installing hooks");
    if claude {
        setup_claude()?;
        println!("  ✓  Claude Code");
    }
    if cursor {
        setup_cursor()?;
        println!("  ✓  Cursor");
    }
    if windsurf {
        setup_windsurf()?;
        println!("  ✓  Windsurf");
    }
    if vscode {
        let _ = setup_vscode();
        let _ = setup_copilot();
        println!("  ✓  VS Code");
    }
    if cont {
        let _ = setup_continue();
        println!("  ✓  Continue");
    }
    if jetbrains {
        let _ = setup_jetbrains();
        println!("  ✓  JetBrains");
    }
    if aider {
        let _ = setup_aider();
        println!("  ✓  Aider");
    }
    if codex {
        let _ = setup_codex();
        println!("  ✓  Codex");
    }
    if !claude && !cursor && !windsurf && !vscode && !cont && !jetbrains && !aider && !codex {
        println!("  ·  none — later: ctx setup all");
    }

    println!();
    println!("Done");
    println!("  ctx app · ctx doctor · ctx snapshot create");
    Ok(())
}

fn mark(ok: bool) -> &'static str {
    if ok {
        "✓"
    } else {
        "·"
    }
}

pub(crate) fn detect_claude() -> bool {
    dirs::home_dir()
        .map(|h| h.join(".claude").exists())
        .unwrap_or(false)
        || Command::new("which")
            .arg("claude")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
}

pub(crate) fn detect_claude_desktop() -> bool {
    dirs::home_dir()
        .map(|h| claude_desktop_dir(&h).is_dir())
        .unwrap_or(false)
}

pub(crate) fn detect_cursor() -> bool {
    Path::new("/Applications/Cursor.app").exists()
        || dirs::home_dir()
            .map(|h| h.join(".cursor").exists())
            .unwrap_or(false)
}

pub(crate) fn detect_windsurf() -> bool {
    Path::new("/Applications/Windsurf.app").exists()
        || dirs::home_dir()
            .map(|h| h.join(".codeium").join("windsurf").exists())
            .unwrap_or(false)
}

pub(crate) fn detect_vscode() -> bool {
    dirs::home_dir()
        .map(|h| {
            h.join("Library")
                .join("Application Support")
                .join("Code")
                .is_dir()
                || h.join(".config").join("Code").is_dir()
                || h.join("AppData").join("Roaming").join("Code").is_dir()
        })
        .unwrap_or(false)
}

pub(crate) fn detect_continue() -> bool {
    dirs::home_dir()
        .map(|h| h.join(".continue").is_dir())
        .unwrap_or(false)
}

pub(crate) fn detect_jetbrains() -> bool {
    dirs::home_dir()
        .map(|h| {
            h.join("Library")
                .join("Application Support")
                .join("JetBrains")
                .is_dir()
                || h.join(".config").join("JetBrains").is_dir()
                || h.join(".local").join("share").join("JetBrains").is_dir()
                || h.join("AppData").join("Roaming").join("JetBrains").is_dir()
        })
        .unwrap_or(false)
}

pub(crate) fn detect_aider() -> bool {
    which("aider")
}

pub(crate) fn detect_codex() -> bool {
    dirs::home_dir()
        .map(|h| h.join(".codex").is_dir())
        .unwrap_or(false)
        || which("codex")
}

fn which(name: &str) -> bool {
    let path = std::env::var_os("PATH").unwrap_or_default();
    std::env::split_paths(&path).any(|dir| {
        let unix = dir.join(name).is_file();
        let win = dir.join(format!("{name}.exe")).is_file();
        unix || win
    })
}

fn ctx_bin() -> String {
    std::env::current_exe()
        .ok()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "ctx".into())
}

fn hook_cmd() -> String {
    hook_cmd_for(&ctx_bin())
}

pub fn hook_cmd_for(bin: &str) -> String {
    let q = if bin.chars().any(|c| c.is_whitespace() || c == '\'') {
        single_quote(bin)
    } else {
        bin.to_string()
    };
    // Fail-open if the binary is missing. Keep the substring "ctx hook" so
    // re-running setup can find and replace this entry.
    format!(
        "if [ -x {q} ]; then {q} hook; else echo 'ctx hook skipped — binary not found. Tool continues without CTX.' >&2; fi"
    )
}

fn setup_claude() -> anyhow::Result<()> {
    let home = dirs::home_dir().context("home directory")?;
    let settings_path = home.join(".claude").join("settings.json");
    fs::create_dir_all(settings_path.parent().unwrap())
        .with_context(|| format!("create {}", settings_path.parent().unwrap().display()))?;
    let mut settings = read_json_object(&settings_path)?;
    merge_claude_hooks(&mut settings, &hook_cmd())
        .with_context(|| format!("merge hooks in {}", settings_path.display()))?;
    merge_mcp(settings.as_object_mut().unwrap(), &ctx_bin())
        .with_context(|| format!("merge mcpServers in {}", settings_path.display()))?;
    write_json_atomic(&settings_path, &settings)?;

    let claude_json = home.join(".claude.json");
    let mut root = read_json_object(&claude_json)?;
    merge_mcp(root.as_object_mut().unwrap(), &ctx_bin())
        .with_context(|| format!("merge mcpServers in {}", claude_json.display()))?;
    write_json_atomic(&claude_json, &root)?;
    println!("Installed Claude Code hooks → {}", settings_path.display());
    Ok(())
}

fn claude_desktop_dir(home: &Path) -> PathBuf {
    if cfg!(target_os = "macos") {
        home.join("Library")
            .join("Application Support")
            .join("Claude")
    } else if cfg!(target_os = "windows") {
        home.join("AppData")
            .join("Roaming")
            .join("Claude")
    } else {
        home.join(".config").join("Claude")
    }
}

pub(crate) fn claude_desktop_config_path(home: &Path) -> PathBuf {
    claude_desktop_dir(home).join("claude_desktop_config.json")
}

fn setup_claude_desktop() -> anyhow::Result<()> {
    let home = dirs::home_dir().context("home directory")?;
    let path = claude_desktop_config_path(&home);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).ok();
    }
    let mut root = read_json_object(&path)?;
    merge_mcp(root.as_object_mut().unwrap(), &ctx_bin())
        .with_context(|| format!("merge mcpServers in {}", path.display()))?;
    write_json_atomic(&path, &root)?;
    println!("Installed Claude Desktop MCP → {}", path.display());
    Ok(())
}

fn setup_cursor() -> anyhow::Result<()> {
    let home = dirs::home_dir().context("home directory")?;
    let hooks_path = home.join(".cursor").join("hooks.json");
    fs::create_dir_all(hooks_path.parent().unwrap())
        .with_context(|| format!("create {}", hooks_path.parent().unwrap().display()))?;
    let mut hooks = read_json_object(&hooks_path)?;
    merge_cursor_hooks(&mut hooks, &hook_cmd())
        .with_context(|| format!("merge hooks in {}", hooks_path.display()))?;
    write_json_atomic(&hooks_path, &hooks)?;

    let mcp_path = home.join(".cursor").join("mcp.json");
    let mut mcp = read_json_object(&mcp_path)?;
    merge_mcp(mcp.as_object_mut().unwrap(), &ctx_bin())
        .with_context(|| format!("merge mcpServers in {}", mcp_path.display()))?;
    write_json_atomic(&mcp_path, &mcp)?;
    println!("Installed Cursor hooks → {}", hooks_path.display());
    if let Ok(cwd) = std::env::current_dir() {
        if cwd.join(".git").exists() || cwd.join(".cursor").exists() {
            setup_cursor_project(&cwd)?;
        }
    }
    Ok(())
}

fn setup_cursor_project(cwd: &std::path::Path) -> anyhow::Result<()> {
    let dir = cwd.join(".cursor");
    fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
    let hooks_path = dir.join("hooks.json");
    let mut hooks = read_json_object(&hooks_path)?;
    merge_cursor_hooks(&mut hooks, &hook_cmd())
        .with_context(|| format!("merge hooks in {}", hooks_path.display()))?;
    write_json_atomic(&hooks_path, &hooks)?;
    println!("Installed project Cursor hooks → {}", hooks_path.display());
    Ok(())
}

fn setup_vscode() -> anyhow::Result<()> {
    let home = dirs::home_dir().context("home directory")?;
    let candidates = [
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
    ];
    let path = candidates
        .iter()
        .find(|p| p.parent().map(|d| d.exists()).unwrap_or(false))
        .cloned()
        .unwrap_or_else(|| candidates[0].clone());
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).ok();
    }
    let mut root = read_json_object(&path)?;
    if !root.is_object() {
        root = json!({});
    }
    let obj = root.as_object_mut().unwrap();
    let servers = obj.entry("servers").or_insert_with(|| json!({}));
    if let Some(map) = servers.as_object_mut() {
        map.insert(
            "ctx".into(),
            json!({"type": "stdio", "command": ctx_bin(), "args": ["mcp"]}),
        );
    }
    write_json_atomic(&path, &root)?;
    println!("Installed VS Code MCP → {}", path.display());
    Ok(())
}

fn setup_windsurf() -> anyhow::Result<()> {
    let home = dirs::home_dir().context("home directory")?;
    let dir = home.join(".codeium").join("windsurf");
    fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
    let mcp_path = dir.join("mcp_config.json");
    let mut mcp = read_json_object(&mcp_path)?;
    merge_mcp(mcp.as_object_mut().unwrap(), &ctx_bin())
        .with_context(|| format!("merge mcpServers in {}", mcp_path.display()))?;
    write_json_atomic(&mcp_path, &mcp)?;
    println!("Installed Windsurf MCP → {}", mcp_path.display());
    Ok(())
}

fn continue_yaml(bin: &str) -> String {
    format!("name: ctx\ncommand: {bin}\nargs:\n  - mcp\n")
}

fn setup_continue() -> anyhow::Result<()> {
    let home = dirs::home_dir().context("home directory")?;
    let dir = home.join(".continue").join("mcpServers");
    fs::create_dir_all(&dir).ok();
    let path = dir.join("ctx.yaml");
    fs::write(&path, continue_yaml(&ctx_bin()))?;
    println!("Installed Continue MCP → {}", path.display());
    Ok(())
}

fn setup_jetbrains() -> anyhow::Result<()> {
    let home = dirs::home_dir().context("home directory")?;
    let candidates = [
        home.join("Library")
            .join("Application Support")
            .join("JetBrains")
            .join("mcp.json"),
        home.join(".config").join("JetBrains").join("mcp.json"),
        home.join("AppData")
            .join("Roaming")
            .join("JetBrains")
            .join("mcp.json"),
    ];
    let mut path = candidates
        .iter()
        .find(|p| p.parent().map(|d| d.exists()).unwrap_or(false))
        .cloned()
        .unwrap_or_else(|| candidates[0].clone());
    if Path::new(".idea").is_dir() {
        path = PathBuf::from(".idea").join("mcp.json");
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).ok();
    }
    write_mcp_servers(&path)?;
    println!("Installed JetBrains MCP → {}", path.display());
    Ok(())
}

fn setup_aider() -> anyhow::Result<()> {
    let paths = CtxPaths::default_home()?;
    let dir = paths.root().join("bin");
    fs::create_dir_all(&dir)?;
    let wrap = dir.join(if cfg!(windows) {
        "aider-ctx.cmd"
    } else {
        "aider-ctx"
    });
    let bin = ctx_bin();
    let body = if cfg!(windows) {
        format!("@echo off\r\n\"{bin}\" exec -- aider %*\r\n")
    } else {
        format!("#!/usr/bin/env bash\nexec {bin} exec -- aider \"$@\"\n")
    };
    fs::write(&wrap, body)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&wrap)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&wrap, perms)?;
    }
    println!("Aider wrapper → {}", wrap.display());
    println!("Use it instead of aider so long command dumps stay in the CTX store.");
    Ok(())
}

fn setup_codex() -> anyhow::Result<()> {
    let home = dirs::home_dir().context("home directory")?;
    let dir = home.join(".codex");
    fs::create_dir_all(&dir).ok();
    let path = dir.join("config.toml");
    let mut text = fs::read_to_string(&path).unwrap_or_default();
    if text.contains("[mcp_servers.ctx]") {
        println!("Codex already has ctx MCP → {}", path.display());
        return Ok(());
    }
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    let bin = ctx_bin().replace('\\', "\\\\").replace('"', "\\\"");
    text.push_str(&format!(
        "\n[mcp_servers.ctx]\ncommand = \"{bin}\"\nargs = [\"mcp\"]\n"
    ));
    fs::write(&path, text)?;
    println!("Installed Codex MCP → {}", path.display());
    Ok(())
}

fn setup_copilot() -> anyhow::Result<()> {
    setup_vscode()?;
    let ws = PathBuf::from(".vscode").join("mcp.json");
    if Path::new(".vscode").is_dir() || Path::new(".git").is_dir() {
        if let Some(parent) = ws.parent() {
            fs::create_dir_all(parent).ok();
        }
        write_vscode_mcp(&ws)?;
        println!("Installed Copilot workspace MCP → {}", ws.display());
    }
    Ok(())
}

fn write_mcp_servers(path: &Path) -> anyhow::Result<()> {
    let mut root = read_json_object(path)?;
    if !root.is_object() {
        root = json!({});
    }
    let obj = root.as_object_mut().unwrap();
    merge_mcp(obj, &ctx_bin())?;
    write_json_atomic(path, &root)
}

fn write_vscode_mcp(path: &Path) -> anyhow::Result<()> {
    let mut root = read_json_object(path)?;
    if !root.is_object() {
        root = json!({});
    }
    let obj = root.as_object_mut().unwrap();
    let servers = obj.entry("servers").or_insert_with(|| json!({}));
    if let Some(map) = servers.as_object_mut() {
        map.insert(
            "ctx".into(),
            json!({"type": "stdio", "command": ctx_bin(), "args": ["mcp"]}),
        );
    }
    write_json_atomic(path, &root)
}

pub fn read_json_object(path: &Path) -> anyhow::Result<Value> {
    if !path.exists() {
        return Ok(json!({}));
    }
    let raw = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let value: Value = serde_json::from_str(&raw)
        .with_context(|| format!("{} is not valid JSON", path.display()))?;
    if !value.is_object() {
        bail!("{}: expected a JSON object", path.display());
    }
    Ok(value)
}

pub fn write_json_atomic(path: &Path, value: &Value) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let mut body = serde_json::to_string_pretty(value)
        .with_context(|| format!("serialize {}", path.display()))?;
    body.push('\n');
    let tmp = tmp_path(path);
    fs::write(&tmp, &body).with_context(|| format!("write {}", tmp.display()))?;
    fs::rename(&tmp, path).with_context(|| {
        let _ = fs::remove_file(&tmp);
        format!("replace {}", path.display())
    })?;
    Ok(())
}

fn tmp_path(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("ctx.json");
    path.with_file_name(format!(".{name}.{}.tmp", std::process::id()))
}

pub fn merge_mcp(obj: &mut serde_json::Map<String, Value>, bin: &str) -> anyhow::Result<()> {
    let entry = json!({
        "command": bin,
        "args": ["mcp"]
    });
    let servers = obj.entry("mcpServers").or_insert_with(|| json!({}));
    let Some(map) = servers.as_object_mut() else {
        bail!("mcpServers must be a JSON object");
    };
    map.insert("ctx".into(), entry);
    Ok(())
}

fn claude_hook_entry(cmd: &str) -> Value {
    json!([{
        "hooks": [{ "type": "command", "command": cmd }]
    }])
}

pub fn merge_claude_hooks(settings: &mut Value, cmd: &str) -> anyhow::Result<()> {
    if !settings.is_object() {
        *settings = json!({});
    }
    let obj = settings.as_object_mut().unwrap();
    let hooks = obj.entry("hooks").or_insert_with(|| json!({}));
    let Some(map) = hooks.as_object_mut() else {
        bail!("hooks must be a JSON object");
    };
    for event in [
        "SessionStart",
        "UserPromptSubmit",
        "PreToolUse",
        "PostToolUse",
        "PostToolUseFailure",
        "PreCompact",
        "PostCompact",
        "SessionEnd",
    ] {
        let entry = claude_hook_entry(cmd);
        replace_or_insert_claude(map, event, entry, cmd)?;
    }
    Ok(())
}

fn replace_or_insert_claude(
    map: &mut serde_json::Map<String, Value>,
    event: &str,
    entry: Value,
    _cmd: &str,
) -> anyhow::Result<()> {
    let existing = map.entry(event).or_insert_with(|| json!([]));
    let Some(arr) = existing.as_array_mut() else {
        map.insert(event.into(), entry);
        return Ok(());
    };
    arr.retain(|item| !hooks_contain_ctx(Some(item)));
    if let Some(items) = entry.as_array() {
        arr.extend(items.iter().cloned());
    }
    Ok(())
}

pub fn merge_cursor_hooks(root: &mut Value, cmd: &str) -> anyhow::Result<()> {
    if !root.is_object() {
        *root = json!({ "version": 1, "hooks": {} });
    }
    let obj = root.as_object_mut().unwrap();
    obj.entry("version").or_insert(json!(1));
    let hooks = obj.entry("hooks").or_insert_with(|| json!({}));
    let Some(map) = hooks.as_object_mut() else {
        bail!("hooks must be a JSON object");
    };
    let entry = json!([{ "command": cmd }]);
    for event in [
        "sessionStart",
        "sessionEnd",
        "preToolUse",
        "postToolUse",
        "postToolUseFailure",
        "beforeReadFile",
        "beforeSubmitPrompt",
        "afterShellExecution",
        "afterMCPExecution",
        "preCompact",
        "subagentStart",
        "afterAgentResponse",
    ] {
        let existing = map.entry(event).or_insert_with(|| json!([]));
        if let Some(arr) = existing.as_array_mut() {
            arr.retain(|item| {
                item.get("command")
                    .and_then(|c| c.as_str())
                    .map(|c| !is_ctx_hook_command(c))
                    .unwrap_or(true)
            });
            arr.extend(entry.as_array().cloned().unwrap_or_default());
        } else {
            map.insert(event.into(), entry.clone());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doctor::{hooks_contain_ctx, is_ctx_hook_command, mcp_registered};
    use serde_json::json;

    #[test]
    fn merge_claude_is_idempotent() {
        let cmd = hook_cmd_for("/opt/ctx");
        let mut settings = json!({"env": {"ANTHROPIC_API_KEY": "sk-secret"}});
        merge_claude_hooks(&mut settings, &cmd).unwrap();
        merge_claude_hooks(&mut settings, &cmd).unwrap();
        merge_mcp(settings.as_object_mut().unwrap(), "/opt/ctx").unwrap();
        merge_mcp(settings.as_object_mut().unwrap(), "/opt/ctx").unwrap();
        let post = settings["hooks"]["PostToolUse"].as_array().unwrap();
        assert_eq!(post.len(), 1, "{post:?}");
        assert!(settings["hooks"]["UserPromptSubmit"].as_array().is_some());
        assert!(settings["hooks"]["PostCompact"].as_array().is_some());
        assert!(hooks_contain_ctx(Some(&settings["hooks"])));
        assert!(mcp_registered(&settings));
        assert_eq!(settings["mcpServers"].as_object().unwrap().len(), 1);
    }

    #[test]
    fn merge_cursor_is_idempotent() {
        let cmd = hook_cmd_for("/opt/ctx");
        let mut hooks = json!({"version": 1, "hooks": {}});
        merge_cursor_hooks(&mut hooks, &cmd).unwrap();
        merge_cursor_hooks(&mut hooks, &cmd).unwrap();
        let pre = hooks["hooks"]["preToolUse"].as_array().unwrap();
        assert_eq!(pre.len(), 1, "{pre:?}");
        assert!(hooks["hooks"]["beforeSubmitPrompt"].as_array().is_some());
        assert!(hooks["hooks"]["afterShellExecution"].as_array().is_some());
        assert!(hooks["hooks"]["afterMCPExecution"].as_array().is_some());
        assert!(is_ctx_hook_command(pre[0]["command"].as_str().unwrap()));
    }

    #[test]
    fn merge_cursor_registers_shell_and_mcp_hooks() {
        let cmd = hook_cmd_for("/opt/ctx");
        let mut hooks = json!({"version": 1, "hooks": {}});
        merge_cursor_hooks(&mut hooks, &cmd).unwrap();
        for event in [
            "afterShellExecution",
            "afterMCPExecution",
            "preCompact",
            "subagentStart",
            "afterAgentResponse",
            "postToolUseFailure",
        ] {
            let arr = hooks["hooks"][event]
                .as_array()
                .unwrap_or_else(|| panic!("missing {event}"));
            assert_eq!(arr.len(), 1, "{event}: {arr:?}");
        }
    }

    #[test]
    fn merge_cursor_project_hooks_json() {
        let dir = tempfile::tempdir().unwrap();
        let cursor = dir.path().join(".cursor");
        std::fs::create_dir_all(&cursor).unwrap();
        setup_cursor_project(dir.path()).unwrap();
        let hooks: Value = serde_json::from_slice(
            &std::fs::read(cursor.join("hooks.json")).unwrap(),
        )
        .unwrap();
        assert!(hooks["hooks"]["preToolUse"].as_array().is_some());
        assert!(hooks["hooks"]["afterShellExecution"].as_array().is_some());
    }

    #[test]
    fn continue_yaml_lists_mcp_stdio() {
        let y = continue_yaml("/opt/ctx");
        assert!(y.contains("command: /opt/ctx"), "{y}");
        assert!(y.contains("- mcp"), "{y}");
    }

    #[test]
    fn merge_mcp_replaces_without_duplicating() {
        let mut obj = serde_json::Map::new();
        merge_mcp(&mut obj, "/opt/ctx").unwrap();
        merge_mcp(&mut obj, "/opt/ctx-new").unwrap();
        let servers = obj["mcpServers"].as_object().unwrap();
        assert_eq!(servers.len(), 1);
        assert_eq!(servers["ctx"]["command"], "/opt/ctx-new");
    }

    #[test]
    fn merge_mcp_rejects_array() {
        let mut obj = serde_json::Map::new();
        obj.insert("mcpServers".into(), json!([]));
        let err = merge_mcp(&mut obj, "/opt/ctx").unwrap_err();
        assert!(err.to_string().contains("mcpServers"), "{err}");
    }

    #[test]
    fn invalid_json_names_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        fs::write(&path, "{ \"api_key\": \"sk-secret-do-not-print\"").unwrap();
        let err = read_json_object(&path).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("settings.json"), "{msg}");
        assert!(
            !msg.contains("sk-secret-do-not-print"),
            "must not dump file body: {msg}"
        );
    }

    #[test]
    fn atomic_write_replaces_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        write_json_atomic(&path, &json!({"a": 1})).unwrap();
        write_json_atomic(&path, &json!({"a": 2})).unwrap();
        let v: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(v["a"], 2);
        let leftovers: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
    }

    #[test]
    fn hook_cmd_fail_open_mentions_ctx_hook() {
        let cmd = hook_cmd_for("/opt/ctx");
        assert!(is_ctx_hook_command(&cmd));
        assert!(cmd.contains("binary not found"));
    }
}
