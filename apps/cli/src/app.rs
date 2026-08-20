use std::fs;
use std::io::ErrorKind;
use std::process::Command;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use anyhow::Context;
use ctx_core::CtxPaths;
use serde_json::{json, Value};

use crate::harnesses;
use crate::setup;
use crate::status;
use crate::uninstall;

const PAGE: &str = include_str!("app.html");
const WORDMARK: &[u8] = include_bytes!("assets/ctx-wordmark.png");
const ARROW_RIGHT: &[u8] = include_bytes!("assets/arrow-right.png");
const SERVICE_LABEL: &str = "ai.ctx.dashboard";

pub fn run(
    port: u16,
    open: bool,
    install: bool,
    uninstall: bool,
    install_app: bool,
) -> anyhow::Result<()> {
    if uninstall {
        return uninstall_service();
    }
    if install_app {
        return install_menubar_app();
    }
    if install {
        return install_service(port);
    }

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .worker_threads(2)
        .thread_name("ctx-app")
        .build()
        .context("tokio runtime")?;
    rt.block_on(serve(port, open))
}

async fn serve(port: u16, open: bool) -> anyhow::Result<()> {
    let addr = format!("127.0.0.1:{port}");
    let url = format!("http://{addr}");
    let listener = match TcpListener::bind(&addr).await {
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
    maybe_auto_snapshot();
    loop {
        let (mut stream, _) = match listener.accept().await {
            Ok(s) => s,
            Err(err) => {
                tracing::warn!(error = %err, "dashboard accept failed");
                continue;
            }
        };
        tokio::spawn(async move {
            let mut buf = vec![0u8; 32_768];
            let n = match stream.read(&mut buf).await {
                Ok(0) | Err(_) => return,
                Ok(n) => n,
            };
            let req = String::from_utf8_lossy(&buf[..n]);
            let (head, req_body) = match req.split_once("\r\n\r\n") {
                Some(pair) => pair,
                None => (req.as_ref(), ""),
            };
            let (method, path, query) = parse_req(head);
            let (status, content_type, body) = dispatch_with(method, path, query, req_body);
            let resp = format!(
                "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(resp.as_bytes()).await;
            let _ = stream.write_all(&body).await;
        });
    }
}

fn maybe_auto_snapshot() {
    let Ok(paths) = ctx_core::CtxPaths::default_home() else {
        return;
    };
    let cfg = ctx_core::Config::load(&paths);
    if !cfg.auto_snapshot {
        return;
    }
    let Ok(rt) = ctx_core::Runtime::open_default() else {
        return;
    };
    let Ok(snaps) = rt.store.list_snapshots() else {
        return;
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    if snaps
        .iter()
        .any(|s| now.saturating_sub(s.created_at) < 86_400)
    {
        return;
    }
    let _ = rt.store.create_snapshot(Some("auto"));
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

#[cfg(test)]
fn dispatch(method: &str, path: &str, query: &str) -> (&'static str, &'static str, Vec<u8>) {
    dispatch_with(method, path, query, "")
}

fn dispatch_with(
    method: &str,
    path: &str,
    query: &str,
    body: &str,
) -> (&'static str, &'static str, Vec<u8>) {
    match (method, path) {
        ("GET", "/") | ("GET", "/index.html") => (
            "200 OK",
            "text/html; charset=utf-8",
            PAGE.as_bytes().to_vec(),
        ),
        ("GET", "/assets/ctx-wordmark.png") => ("200 OK", "image/png", WORDMARK.to_vec()),
        ("GET", "/assets/arrow-right.png") => ("200 OK", "image/png", ARROW_RIGHT.to_vec()),
        ("GET", "/api/status") => {
            let range = match query_param(query, "range") {
                "" => "7d",
                r => r,
            };
            let model = match query_param(query, "model") {
                "" => "all",
                m => m,
            };
            let from = query_param(query, "from").parse().ok();
            let to = query_param(query, "to").parse().ok();
            json_ok(dashboard_payload(range, model, from, to))
        }
        ("GET", "/api/harnesses") => json_ok(harnesses::payload()),
        ("POST", "/api/harness") => json_ok(harnesses::set_enabled(
            query_param(query, "id"),
            query_param(query, "enabled") != "0",
        )),
        ("GET", "/api/doctor") => json_ok(doctor_payload()),
        ("GET", "/api/health") => json_ok(health_payload()),
        ("POST", "/api/prices/refresh") => json_ok(prices_refresh()),
        ("POST", "/api/uninstall") => json_ok(uninstall_target(query_param(query, "target"))),
        ("POST", "/api/setup") => json_ok(setup_target(query_param(query, "target"))),
        ("POST", "/api/pause") => json_ok(set_enabled(false)),
        ("POST", "/api/resume") => json_ok(set_enabled(true)),
        ("POST", "/api/snapshot") => json_ok(snapshot_create()),
        ("POST", "/api/snapshot/restore") => json_ok(snapshot_restore(query_param(query, "id"))),
        ("GET", "/api/config") => json_ok(config_get()),
        ("POST", "/api/config") => json_ok(config_set(body)),
        ("GET", "/api/pages") => json_ok(pages_payload()),
        ("GET", "/api/inspect") => json_ok(inspect_payload()),
        ("GET", "/metrics") => (
            "200 OK",
            "text/plain; version=0.0.4; charset=utf-8",
            prometheus_text(),
        ),
        _ => ("404 Not Found", "text/plain", b"not found".to_vec()),
    }
}

fn config_get() -> serde_json::Value {
    match ctx_core::CtxPaths::default_home() {
        Ok(paths) => {
            let cfg = ctx_core::Config::load(&paths);
            let mut value = serde_json::to_value(&cfg).unwrap_or(json!({"ok": false}));
            if let Some(obj) = value.as_object_mut() {
                obj.insert("ok".into(), json!(true));
                obj.insert("price_catalog".into(), ctx_core::catalog_json());
                let (entries, fetched_at) = ctx_core::official_price_meta(&paths);
                obj.insert("price_entries".into(), json!(entries));
                obj.insert("price_fetched_at".into(), json!(fetched_at));
            }
            value
        }
        Err(err) => json!({"ok": false, "error": err.to_string()}),
    }
}

fn config_set(body: &str) -> serde_json::Value {
    let Ok(paths) = ctx_core::CtxPaths::default_home() else {
        return json!({"ok": false, "error": "no home"});
    };
    let mut cfg = ctx_core::Config::load(&paths);
    let Ok(v) = serde_json::from_str::<serde_json::Value>(body) else {
        return json!({"ok": false, "error": "invalid json"});
    };
    if let Some(b) = v.get("enabled").and_then(|x| x.as_bool()) {
        cfg.enabled = b;
    }
    if let Some(s) = v.get("budget_strategy").and_then(|x| x.as_str()) {
        cfg.budget_strategy = s.to_string();
    }
    if let Some(n) = v
        .get("virtualize_threshold_tokens")
        .and_then(|x| x.as_u64())
    {
        cfg.virtualize_threshold_tokens = n as u32;
    }
    if let Some(n) = v.get("large_file_tokens").and_then(|x| x.as_u64()) {
        cfg.large_file_tokens = n as u32;
    }
    if let Some(b) = v.get("dashboard_autostart").and_then(|x| x.as_bool()) {
        cfg.dashboard_autostart = b;
        if b {
            let _ = install_service(8741);
        } else {
            let _ = uninstall_service();
        }
    }
    if let Some(b) = v.get("auto_snapshot").and_then(|x| x.as_bool()) {
        cfg.auto_snapshot = b;
    }
    if let Some(s) = v.get("default_billing_model").and_then(|x| x.as_str()) {
        cfg.default_billing_model = s.trim().to_string();
    }
    if let Some(b) = v.get("shadow_mode").and_then(|x| x.as_bool()) {
        cfg.shadow_mode = b;
    }
    if let Some(arr) = v.get("disabled_harnesses").and_then(|x| x.as_array()) {
        cfg.disabled_harnesses = arr
            .iter()
            .filter_map(|x| x.as_str().map(str::to_string))
            .collect();
    }
    if let Some(arr) = v.get("shadow_harnesses").and_then(|x| x.as_array()) {
        cfg.shadow_harnesses = arr
            .iter()
            .filter_map(|x| x.as_str().map(str::to_string))
            .collect();
    }
    match cfg.save(&paths) {
        Ok(()) => json!({"ok": true}),
        Err(err) => json!({"ok": false, "error": err.to_string()}),
    }
}

fn pages_payload() -> serde_json::Value {
    match ctx_core::Runtime::open_default() {
        Ok(rt) => match rt.store.recent_pages(48) {
            Ok(pages) => json!({
                "ok": true,
                "pages": pages.iter().map(|p| json!({
                    "uri": p.uri,
                    "kind": p.kind,
                    "summary": p.summary,
                    "raw_tokens": p.raw_tokens,
                    "task": p.task,
                    "harness": p.harness,
                    "created_at": p.created_at,
                })).collect::<Vec<_>>()
            }),
            Err(err) => json!({"ok": false, "error": err.to_string()}),
        },
        Err(err) => json!({"ok": false, "error": err.to_string()}),
    }
}

fn inspect_payload() -> serde_json::Value {
    match ctx_core::Runtime::open_default() {
        Ok(rt) => match ctx_core::WorkingSet::query(&rt.store, None, &[]) {
            Ok(ws) => json!({
                "ok": true,
                "task": ws.task,
                "pages": ws.recent_pages.iter().map(|p| json!({
                    "uri": p.uri,
                    "layer": p.layer,
                    "label": p.label,
                    "tokens": p.tokens,
                    "harness": p.harness,
                    "frame": p.frame,
                })).collect::<Vec<_>>()
            }),
            Err(err) => json!({"ok": false, "error": err.to_string()}),
        },
        Err(err) => json!({"ok": false, "error": err.to_string()}),
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

fn dashboard_payload(range: &str, model: &str, from: Option<i64>, to: Option<i64>) -> Value {
    match status::dashboard(range, model, from, to) {
        Ok(mut v) => {
            if let Some(obj) = v.as_object_mut() {
                obj.insert("harness_summary".into(), harnesses::summary_json());
                obj.insert("tools".into(), harnesses::payload()["harnesses"].clone());
            }
            v
        }
        Err(err) => json!({"ok": false, "error": err.to_string()}),
    }
}

fn doctor_payload() -> Value {
    match crate::doctor::collect() {
        Ok(report) => json!({
            "ok": true,
            "checks": report.checks.iter().map(|c| json!({
                "ok": c.ok,
                "name": c.name,
                "detail": c.detail,
            })).collect::<Vec<_>>()
        }),
        Err(err) => json!({"ok": false, "error": err.to_string()}),
    }
}

fn health_payload() -> Value {
    let (p50, p95, _) = ctx_core::hook_latency_ms();
    let (enabled, shadow, intercepts) = match ctx_core::Runtime::open_default() {
        Ok(rt) => {
            let intercepts = rt
                .store
                .observation_count_since(ctx_core::start_of_today())
                .unwrap_or(0);
            (rt.config.enabled, rt.config.shadow_mode, intercepts)
        }
        Err(_) => (true, false, 0),
    };
    json!({
        "ok": true,
        "enabled": enabled,
        "shadow": shadow,
        "hook_p50_ms": (p50 * 10.0).round() / 10.0,
        "hook_p95_ms": (p95 * 10.0).round() / 10.0,
        "intercepts_today": intercepts,
    })
}

fn prices_refresh() -> Value {
    let Ok(paths) = ctx_core::CtxPaths::default_home() else {
        return json!({"ok": false, "error": "no home"});
    };
    let _ = ctx_core::refresh_official_prices_now(&paths);
    let (entries, fetched_at) = ctx_core::official_price_meta(&paths);
    json!({"ok": true, "entries": entries, "fetched_at": fetched_at})
}

fn uninstall_target(target: &str) -> Value {
    if target.is_empty() {
        return json!({"ok": false, "error": "missing target"});
    }
    match uninstall::strip_target(target) {
        Ok(()) => json!({"ok": true, "target": target}),
        Err(err) => json!({"ok": false, "error": err.to_string()}),
    }
}

fn setup_target(target: &str) -> Value {
    let result = match target {
        "claude" | "claude-code" => setup::setup("claude"),
        "claude-desktop" => setup::setup("claude-desktop"),
        "cursor" => setup::setup("cursor"),
        "windsurf" => setup::setup("windsurf"),
        "vscode" | "code" => setup::setup("vscode"),
        "continue" => setup::setup("continue"),
        "jetbrains" | "idea" => setup::setup("jetbrains"),
        "aider" => setup::setup("aider"),
        "codex" => setup::setup("codex"),
        "copilot" => setup::setup("copilot"),
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

fn install_menubar_app() -> anyhow::Result<()> {
    #[cfg(not(target_os = "macos"))]
    {
        anyhow::bail!("menu bar app is macOS only. Use: ctx app");
    }
    #[cfg(target_os = "macos")]
    {
        let dest = dirs::home_dir()
            .context("home directory")?
            .join("Applications/CTX.app");
        if let Some(built) = find_menubar_bundle() {
            fs::create_dir_all(dest.parent().unwrap())?;
            if dest.exists() {
                fs::remove_dir_all(&dest)?;
            }
            copy_dir(&built, &dest)?;
            let _ = Command::new("open").arg(&dest).status();
            println!("CTX menu bar  {}", dest.display());
            println!("Look for ↓% next to the clock. Click it for today's avoided tokens.");
            return Ok(());
        }
        let script = find_menubar_build().context(
            "menu bar sources not found. From the repo: bash apps/macos/build.sh --install",
        )?;
        let status = Command::new("bash")
            .arg(&script)
            .arg("--install")
            .status()
            .context("build macOS app")?;
        if !status.success() {
            anyhow::bail!("apps/macos/build.sh failed (needs Xcode CLT + swiftc)");
        }
        Ok(())
    }
}

#[cfg(target_os = "macos")]
fn find_menubar_build() -> Option<std::path::PathBuf> {
    menubar_roots()
        .into_iter()
        .map(|root| root.join("build.sh"))
        .find(|p| p.is_file())
}

#[cfg(target_os = "macos")]
fn find_menubar_bundle() -> Option<std::path::PathBuf> {
    menubar_roots()
        .into_iter()
        .map(|root| root.join("dist/CTX.app"))
        .find(|p| p.join("Contents/MacOS/CTX").is_file())
}

#[cfg(target_os = "macos")]
fn menubar_roots() -> Vec<std::path::PathBuf> {
    let mut roots = Vec::new();
    if let Ok(cwd) = std::env::current_dir() {
        roots.push(cwd.join("apps/macos"));
        let mut cur = cwd;
        for _ in 0..6 {
            roots.push(cur.join("apps/macos"));
            if !cur.pop() {
                break;
            }
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        let mut cur = exe;
        for _ in 0..8 {
            if !cur.pop() {
                break;
            }
            roots.push(cur.join("apps/macos"));
        }
    }
    if let Some(home) = dirs::home_dir() {
        roots.push(home.join(".ctx/src/apps/macos"));
    }
    roots
}

#[cfg(target_os = "macos")]
fn copy_dir(src: &std::path::Path, dest: &std::path::Path) -> anyhow::Result<()> {
    fs::create_dir_all(dest)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let to = dest.join(entry.file_name());
        let ty = entry.file_type()?;
        if ty.is_dir() {
            copy_dir(&entry.path(), &to)?;
        } else if ty.is_file() {
            fs::copy(entry.path(), to)?;
        }
    }
    Ok(())
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
    fn dashboard_renders_the_approved_brand_and_savings_language() {
        let (status, content_type, body) = dispatch("GET", "/", "");
        assert_eq!(status, "200 OK");
        assert!(content_type.contains("text/html"));
        let page = String::from_utf8(body).expect("dashboard html is utf-8");

        assert!(page.contains("让重要的，自然抵达。"));
        assert!(!page.contains("「让重要的"));
        assert!(!page.contains("brand-rule"));
        assert!(page.contains("已节省"));
        assert!(page.contains("/api/status"));
        assert!(page.contains("上下文趋势"));
        assert!(page.contains("range-trigger"));
        assert!(page.contains("当天"));
        assert!(!page.contains("按模型"));
        assert!(!page.contains("最近命中"));
        assert!(page.contains("settings-btn"));
        assert!(page.contains("已接入"));
        assert!(page.contains("/assets/ctx-wordmark.png"));
        assert!(page.contains("id=\"toast\""));
        assert!(!page.contains("影子模式"));
        assert!(!page.contains("卸载 CTX"));
        assert!(!page.contains("cfg-shadow"));
        assert!(!page.contains("uninstall-btn"));
        assert!(!page.contains("春庭雪"));
        assert!(!page.contains("春雪留痕"));
        assert!(!page.contains("API 等价"));
        assert!(!page.contains("按公开输入价估算"));
        assert!(!page.contains("已避免"));
        assert!(!page.contains("自动优化中"));
        assert!(!page.contains(">高级<"));
        assert!(!page.contains("显示未安装"));
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
    fn dashboard_serves_the_ctx_brand_asset() {
        let (status, content_type, body) = dispatch("GET", "/assets/ctx-wordmark.png", "");
        assert_eq!(status, "200 OK");
        assert_eq!(content_type, "image/png");
        assert!(body.starts_with(b"\x89PNG\r\n\x1a\n"));
        assert!(body.len() > 1_000, "brand asset is unexpectedly small");

        let (status, content_type, body) = dispatch("GET", "/assets/arrow-right.png", "");
        assert_eq!(status, "200 OK");
        assert_eq!(content_type, "image/png");
        assert!(body.starts_with(b"\x89PNG\r\n\x1a\n"));
    }

    #[test]
    fn settings_apis_are_json_ok() {
        for path in ["/api/harnesses", "/api/doctor", "/api/health"] {
            let (status, content_type, body) = dispatch("GET", path, "");
            assert_eq!(status, "200 OK", "{path}");
            assert!(content_type.contains("json"), "{path}");
            let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(v["ok"], true, "{path}: {v}");
        }
        let (status, _, body) = dispatch("GET", "/api/harnesses", "");
        assert_eq!(status, "200 OK");
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let ids: Vec<_> = v["harnesses"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|h| h["id"].as_str())
            .collect();
        assert!(ids.contains(&"cursor"), "{ids:?}");
        assert!(ids.contains(&"claude-code"), "{ids:?}");
        assert!(ids.contains(&"codex"), "{ids:?}");
        assert!(!ids.contains(&"claude-desktop"), "{ids:?}");
        assert!(!ids.contains(&"windsurf"), "{ids:?}");
        assert_eq!(ids.len(), 3, "{ids:?}");
        let codex = v["harnesses"]
            .as_array()
            .unwrap()
            .iter()
            .find(|h| h["id"] == "codex")
            .expect("codex row");
        assert_eq!(codex["name"], "ChatGPT");
        assert_eq!(codex["capability"], "auto");
        assert_eq!(codex["form"], "desktop+cli");
        assert_eq!(codex["form_label"], "Desktop / CLI");
        assert_eq!(codex["integration"], "hooks");
    }
}
