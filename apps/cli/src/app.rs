use std::fs;
use std::io::{ErrorKind, Read, Write};
use std::net::TcpListener;
use std::process::Command;

use anyhow::Context;
use ctx_core::CtxPaths;
use serde_json::{json, Value};

use crate::setup;
use crate::status;

const PAGE: &str = include_str!("app.html");
const WORDMARK: &[u8] = include_bytes!("assets/ctx-wordmark.png");
const SERVICE_LABEL: &str = "ai.ctx.dashboard";

pub fn run(port: u16, open: bool, install: bool, uninstall: bool) -> anyhow::Result<()> {
    if uninstall {
        return uninstall_service();
    }
    if install {
        return install_service(port);
    }

    let addr = format!("127.0.0.1:{port}");
    let url = format!("http://{addr}");
    let listener = match TcpListener::bind(&addr) {
        Ok(l) => l,
        Err(err) if err.kind() == ErrorKind::AddrInUse => {
            eprintln!("CTX dashboard already running  {url}");
            if open {
                open_browser(&url);
            }
            return Ok(());
        }
        Err(err) => return Err(err).context(format!("bind {addr}")),
    };
    eprintln!("CTX dashboard  {url}");
    eprintln!("Leave this running. Ctrl+C to stop.");
    if open {
        open_browser(&url);
    }
    for conn in listener.incoming() {
        let mut stream = match conn {
            Ok(s) => s,
            Err(err) => {
                tracing::warn!(error = %err, "dashboard accept failed");
                continue;
            }
        };
        let mut buf = vec![0u8; 16_384];
        let n = match stream.read(&mut buf) {
            Ok(0) | Err(_) => continue,
            Ok(n) => n,
        };
        let req = String::from_utf8_lossy(&buf[..n]);
        let (method, path, query) = parse_req(&req);
        let (status, content_type, body) = dispatch(method, path, query);
        let resp = format!(
            "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
            body.len()
        );
        let _ = stream.write_all(resp.as_bytes());
        let _ = stream.write_all(&body);
    }
    Ok(())
}

fn parse_req(req: &str) -> (&str, &str, &str) {
    let line = req.lines().next().unwrap_or("");
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or("GET");
    let target = parts.next().unwrap_or("/");
    match target.split_once('?') {
        Some((path, query)) => (method, path, query),
        None => (method, target, ""),
    }
}

fn query_param<'a>(query: &'a str, key: &str) -> &'a str {
    query
        .split('&')
        .find_map(|pair| {
            let (k, v) = pair.split_once('=')?;
            (k == key).then_some(v)
        })
        .unwrap_or("")
}

fn dispatch(method: &str, path: &str, query: &str) -> (&'static str, &'static str, Vec<u8>) {
    match (method, path) {
        ("GET", "/") | ("GET", "/index.html") => (
            "200 OK",
            "text/html; charset=utf-8",
            PAGE.as_bytes().to_vec(),
        ),
        ("GET", "/assets/ctx-wordmark.png") => ("200 OK", "image/png", WORDMARK.to_vec()),
        ("GET", "/api/status") => {
            let range = match query_param(query, "range") {
                "" => "7d",
                r => r,
            };
            let model = match query_param(query, "model") {
                "" => "all",
                m => m,
            };
            json_ok(dashboard_payload(range, model))
        }
        ("POST", "/api/setup") => json_ok(setup_target(query_param(query, "target"))),
        ("POST", "/api/pause") => json_ok(set_enabled(false)),
        ("POST", "/api/resume") => json_ok(set_enabled(true)),
        ("POST", "/api/snapshot") => json_ok(snapshot_create()),
        ("POST", "/api/snapshot/restore") => json_ok(snapshot_restore(query_param(query, "id"))),
        ("GET", "/metrics") => (
            "200 OK",
            "text/plain; version=0.0.4; charset=utf-8",
            prometheus_text(),
        ),
        _ => ("404 Not Found", "text/plain", b"not found".to_vec()),
    }
}

fn prometheus_text() -> Vec<u8> {
    match ctx_core::Runtime::open_default() {
        Ok(rt) => rt
            .store
            .prometheus_text()
            .unwrap_or_else(|err| format!("# error {err}\n"))
            .into_bytes(),
        Err(err) => format!("# ctx store unavailable: {err}\n").into_bytes(),
    }
}

fn json_ok(value: Value) -> (&'static str, &'static str, Vec<u8>) {
    ("200 OK", "application/json", value.to_string().into_bytes())
}

fn dashboard_payload(range: &str, model: &str) -> Value {
    match status::dashboard(range, model) {
        Ok(v) => v,
        Err(err) => json!({"ok": false, "error": err.to_string()}),
    }
}

fn setup_target(target: &str) -> Value {
    let result = match target {
        "claude" | "claude-code" => setup::setup("claude"),
        "cursor" => setup::setup("cursor"),
        "windsurf" => setup::setup("windsurf"),
        _ => setup::init(),
    };
    match result {
        Ok(()) => json!({"ok": true, "message": "已安装钩子"}),
        Err(err) => json!({"ok": false, "error": err.to_string()}),
    }
}

fn snapshot_create() -> Value {
    match ctx_core::Runtime::open_default() {
        Ok(rt) => match rt.store.create_snapshot(Some("dashboard")) {
            Ok(s) => json!({"ok": true, "id": s.id}),
            Err(err) => json!({"ok": false, "error": err.to_string()}),
        },
        Err(err) => json!({"ok": false, "error": err.to_string()}),
    }
}

fn snapshot_restore(id: &str) -> Value {
    if id.is_empty() {
        return json!({"ok": false, "error": "missing snapshot id"});
    }
    match ctx_core::Runtime::open_default() {
        Ok(rt) => match rt.store.restore_snapshot(id) {
            Ok(()) => json!({"ok": true, "id": id}),
            Err(err) => json!({"ok": false, "error": err.to_string()}),
        },
        Err(err) => json!({"ok": false, "error": err.to_string()}),
    }
}

fn set_enabled(enabled: bool) -> Value {
    match status::set_enabled(enabled) {
        Ok(()) => json!({"ok": true, "enabled": enabled}),
        Err(err) => json!({"ok": false, "error": err.to_string()}),
    }
}

fn open_browser(url: &str) {
    let _ = if cfg!(target_os = "macos") {
        Command::new("open").arg(url).status()
    } else if cfg!(target_os = "windows") {
        Command::new("cmd").args(["/C", "start", url]).status()
    } else {
        Command::new("xdg-open").arg(url).status()
    };
}

fn ctx_bin() -> anyhow::Result<std::path::PathBuf> {
    std::env::current_exe().context("current ctx binary")
}

fn install_service(port: u16) -> anyhow::Result<()> {
    let exe = ctx_bin()?;
    let paths = CtxPaths::default_home()?;
    fs::create_dir_all(paths.root())?;
    install_service_for(port, &exe, &paths)
}

#[cfg(target_os = "macos")]
fn install_service_for(port: u16, exe: &std::path::Path, paths: &CtxPaths) -> anyhow::Result<()> {
    let log = paths.root().join("dashboard.log");
    let home = dirs::home_dir().context("home directory")?;
    let agents = home.join("Library/LaunchAgents");
    fs::create_dir_all(&agents)?;
    let plist_path = agents.join(format!("{SERVICE_LABEL}.plist"));
    let plist = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>{SERVICE_LABEL}</string>
  <key>ProgramArguments</key>
  <array>
    <string>{}</string>
    <string>app</string>
    <string>--port</string>
    <string>{port}</string>
    <string>--no-open</string>
  </array>
  <key>RunAtLoad</key>
  <true/>
  <key>KeepAlive</key>
  <true/>
  <key>StandardOutPath</key>
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
        .context("launchctl load")?;
    if !status.success() {
        anyhow::bail!("launchctl load failed");
    }
    println!("CTX dashboard starts at login.");
    println!("http://127.0.0.1:{port}");
    Ok(())
}

#[cfg(target_os = "linux")]
fn install_service_for(port: u16, exe: &std::path::Path, _paths: &CtxPaths) -> anyhow::Result<()> {
    let home = dirs::home_dir().context("home directory")?;
    let unit_dir = home.join(".config/systemd/user");
    fs::create_dir_all(&unit_dir)?;
    let unit_path = unit_dir.join("ctx-dashboard.service");
    let unit = format!(
        "[Unit]\nDescription=CTX local dashboard\n\n[Service]\nExecStart={} app --port {port} --no-open\nRestart=on-failure\n\n[Install]\nWantedBy=default.target\n",
        exe.display()
    );
    fs::write(&unit_path, unit)?;
    let _ = Command::new("systemctl")
        .args(["--user", "daemon-reload"])
        .status();
    let status = Command::new("systemctl")
        .args(["--user", "enable", "--now", "ctx-dashboard.service"])
        .status()
        .context("systemctl enable")?;
    if !status.success() {
        anyhow::bail!("systemctl --user enable failed");
    }
    println!("CTX dashboard starts with your session.");
    println!("http://127.0.0.1:{port}");
    Ok(())
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn install_service_for(
    _port: u16,
    _exe: &std::path::Path,
    _paths: &CtxPaths,
) -> anyhow::Result<()> {
    anyhow::bail!("login service is macOS/Linux only. Run: ctx app")
}

#[cfg(target_os = "macos")]
fn uninstall_service() -> anyhow::Result<()> {
    if let Some(home) = dirs::home_dir() {
        let plist = home
            .join("Library/LaunchAgents")
            .join(format!("{SERVICE_LABEL}.plist"));
        let _ = Command::new("launchctl")
            .args(["unload", &plist.to_string_lossy()])
            .status();
        if plist.exists() {
            fs::remove_file(&plist)?;
        }
    }
    println!("CTX dashboard will not start at login.");
    Ok(())
}

#[cfg(target_os = "linux")]
fn uninstall_service() -> anyhow::Result<()> {
    let _ = Command::new("systemctl")
        .args(["--user", "disable", "--now", "ctx-dashboard.service"])
        .status();
    if let Some(home) = dirs::home_dir() {
        let unit = home.join(".config/systemd/user/ctx-dashboard.service");
        if unit.exists() {
            fs::remove_file(unit)?;
        }
    }
    println!("CTX dashboard will not start with your session.");
    Ok(())
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn uninstall_service() -> anyhow::Result<()> {
    anyhow::bail!("nothing to uninstall")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_status_route() {
        assert_eq!(
            parse_req("GET /api/status?range=7d HTTP/1.1\r\nHost: 127.0.0.1\r\n"),
            ("GET", "/api/status", "range=7d")
        );
    }

    #[test]
    fn dashboard_shows_hero_copy() {
        assert!(PAGE.contains("让上下文，精准抵达"));
        assert!(PAGE.contains("/api/status"));
        assert!(PAGE.contains("上下文趋势"));
        assert!(PAGE.contains("优化器拆分"));
        assert!(PAGE.contains("创建快照"));
        assert!(PAGE.contains("实时日志"));
    }

    #[test]
    fn metrics_endpoint_is_prometheus_text() {
        let (status, content_type, body) = dispatch("GET", "/metrics", "");
        assert_eq!(status, "200 OK");
        assert!(content_type.contains("text/plain"));
        let text = String::from_utf8_lossy(&body);
        assert!(
            text.contains("ctx_store") || text.contains("# ctx store") || text.contains("# error"),
            "{text}"
        );
    }

    #[test]
    fn dashboard_serves_the_brand_asset() {
        let (status, content_type, body) = dispatch("GET", "/assets/ctx-wordmark.png", "");
        assert_eq!(status, "200 OK");
        assert_eq!(content_type, "image/png");
        assert!(body.starts_with(b"\x89PNG\r\n\x1a\n"));
        assert!(body.len() > 1_000, "brand asset is unexpectedly small");
    }
}
