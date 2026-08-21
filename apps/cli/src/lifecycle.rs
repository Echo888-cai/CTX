use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::Context;
use serde_json::{json, Value};

use ctx_core::CtxPaths;

use crate::setup;
use crate::status;
use crate::uninstall;

const WATCHDOG_LABEL: &str = "ai.ctx.watchdog";
const BUNDLE_FILE: &str = "app-bundle";

/// Open the app: write IDE hooks, resume saving, watch for deletion.
pub fn activate(bundle: Option<&str>) -> anyhow::Result<Value> {
    let _ = setup::ensure_store()?;
    if let Some(path) = bundle.filter(|s| !s.is_empty()) {
        remember_bundle(path)?;
    } else if let Ok(path) = std::env::var("CTX_APP_BUNDLE") {
        if !path.trim().is_empty() {
            remember_bundle(path.trim())?;
        }
    }
    let wired = match setup::wire_detected() {
        Ok(wired) => wired,
        Err(err) => {
            tracing::warn!(error = %err, "auto-wire detected IDEs failed");
            Vec::new()
        }
    };
    status::set_enabled(true)?;
    if let Err(err) = install_watchdog() {
        tracing::warn!(error = %err, "CTX watchdog not installed");
    }
    Ok(json!({
        "ok": true,
        "wired": wired,
        "enabled": true,
    }))
}

/// Quit the app: pause saving, keep hooks so the next open is instant.
pub fn deactivate() -> anyhow::Result<Value> {
    status::set_enabled(false)?;
    Ok(json!({ "ok": true, "enabled": false }))
}

/// Delete the app: take CTX out of IDE configs.
pub fn detach() -> anyhow::Result<Value> {
    uninstall::strip_harnesses()?;
    status::set_enabled(false)?;
    let _ = uninstall_watchdog();
    let _ = clear_bundle();
    Ok(json!({ "ok": true, "restored": true }))
}

/// LaunchAgent entry: if CTX.app is gone, restore IDE configs.
pub fn sweep() -> anyhow::Result<Value> {
    if !sweep_needed(bundle_path().as_deref(), ctx_app_is_running()) {
        return Ok(json!({ "ok": true, "changed": false }));
    }
    detach()
}

pub(crate) fn sweep_needed(bundle: Option<&Path>, app_running: bool) -> bool {
    let Some(path) = bundle else {
        return false;
    };
    if app_running {
        return false;
    }
    path_is_trashed(path) || !path.exists()
}

pub(crate) fn path_is_trashed(path: &Path) -> bool {
    path.components().any(|c| {
        let name = c.as_os_str();
        name == ".Trash" || name == "Trash"
    })
}

fn remember_bundle(path: &str) -> anyhow::Result<()> {
    let paths = CtxPaths::default_home()?;
    paths.ensure().ok();
    let recorded = PathBuf::from(path);
    let body = recorded
        .canonicalize()
        .unwrap_or(recorded)
        .to_string_lossy()
        .into_owned();
    fs::write(paths.root().join(BUNDLE_FILE), format!("{body}\n"))
        .with_context(|| format!("write {}/{BUNDLE_FILE}", paths.root().display()))?;
    Ok(())
}

fn bundle_path() -> Option<PathBuf> {
    let paths = CtxPaths::default_home().ok()?;
    let text = fs::read_to_string(paths.root().join(BUNDLE_FILE)).ok()?;
    let line = text.lines().next()?.trim();
    if line.is_empty() {
        None
    } else {
        Some(PathBuf::from(line))
    }
}

fn clear_bundle() -> anyhow::Result<()> {
    let paths = CtxPaths::default_home()?;
    let file = paths.root().join(BUNDLE_FILE);
    if file.exists() {
        fs::remove_file(file)?;
    }
    Ok(())
}

fn ctx_app_is_running() -> bool {
    Command::new("pgrep")
        .args(["-qx", "CTX"])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

#[cfg(target_os = "macos")]
fn install_watchdog() -> anyhow::Result<()> {
    let exe = std::env::current_exe().context("current ctx binary")?;
    let paths = CtxPaths::default_home()?;
    let home = dirs::home_dir().context("home directory")?;
    let agents = home.join("Library/LaunchAgents");
    fs::create_dir_all(&agents)?;
    let plist_path = agents.join(format!("{WATCHDOG_LABEL}.plist"));
    let log = paths.root().join("watchdog.log");
    let mut watch = String::new();
    if let Some(bundle) = bundle_path() {
        watch = format!(
            "  <key>WatchPaths</key>\n  <array>\n    <string>{}</string>\n  </array>\n",
            bundle.display()
        );
    }
    let plist = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>{WATCHDOG_LABEL}</string>
  <key>ProgramArguments</key>
  <array>
    <string>{}</string>
    <string>lifecycle</string>
    <string>sweep</string>
  </array>
  <key>RunAtLoad</key>
  <true/>
  <key>StartInterval</key>
  <integer>120</integer>
{watch}  <key>StandardOutPath</key>
  <string>{}</string>
  <key>StandardErrorPath</key>
  <string>{}</string>
</dict>
</plist>
"#,
        exe.display(),
        log.display(),
        log.display()
    );
    fs::write(&plist_path, plist)?;
    let _ = Command::new("launchctl")
        .args(["unload", &plist_path.to_string_lossy()])
        .status();
    let status = Command::new("launchctl")
        .args(["load", "-w", &plist_path.to_string_lossy()])
        .status()
        .context("launchctl load watchdog")?;
    if !status.success() {
        anyhow::bail!("launchctl load {WATCHDOG_LABEL} failed");
    }
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn install_watchdog() -> anyhow::Result<()> {
    Ok(())
}

#[cfg(target_os = "macos")]
fn uninstall_watchdog() -> anyhow::Result<()> {
    let Some(home) = dirs::home_dir() else {
        return Ok(());
    };
    let plist = home
        .join("Library/LaunchAgents")
        .join(format!("{WATCHDOG_LABEL}.plist"));
    let _ = Command::new("launchctl")
        .args(["unload", &plist.to_string_lossy()])
        .status();
    if plist.exists() {
        fs::remove_file(&plist)?;
    }
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn uninstall_watchdog() -> anyhow::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trash_paths_are_detected() {
        assert!(path_is_trashed(Path::new(
            "/Users/ada/.Trash/CTX.app"
        )));
        assert!(!path_is_trashed(Path::new("/Applications/CTX.app")));
    }

    #[test]
    fn sweep_waits_until_the_app_is_actually_gone() {
        let missing = Path::new("/tmp/ctx-app-does-not-exist.app");
        assert!(sweep_needed(Some(missing), false));
        assert!(!sweep_needed(Some(missing), true));
        assert!(!sweep_needed(None, false));
        let existing = Path::new("/");
        assert!(!sweep_needed(Some(existing), false));
    }
}
