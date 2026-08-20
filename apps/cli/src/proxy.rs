//! Intercept plane: Anthropic/OpenAI-compatible proxy.
//!
//! Freezes L0/L1 into `system`, optionally replaces tools with the five CTX
//! capability tools, executes those tools locally, then forwards.

use std::io::Write as _;

use anyhow::Context;
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use ctx_core::Runtime;

pub async fn run(bind: &str, upstream: Option<String>, capability: bool, dry_run: bool) -> anyhow::Result<()> {
    let listener = TcpListener::bind(bind).await.context("bind proxy")?;
    tracing::info!(%bind, ?upstream, capability, dry_run, "CTX intercept plane listening");
    loop {
        let (stream, _) = listener.accept().await?;
        let upstream = upstream.clone();
        tokio::spawn(async move {
            if let Err(err) = handle(stream, upstream.as_deref(), capability, dry_run).await {
                tracing::warn!(error = %err, "proxy request failed");
            }
        });
    }
}

async fn handle(
    mut stream: TcpStream,
    upstream: Option<&str>,
    capability: bool,
    dry_run: bool,
) -> anyhow::Result<()> {
    let (head, body) = read_http(&mut stream).await?;
    if head.is_empty() {
        return Ok(());
    }
    let Some((method, path)) = request_line(&head) else {
        write_http(&mut stream, 400, "text/plain", b"bad request").await?;
        return Ok(());
    };
    if method == "GET" && (path == "/health" || path == "/") {
        write_http(
            &mut stream,
            200,
            "application/json",
            br#"{"ok":true,"plane":"intercept"}"#,
        )
        .await?;
        return Ok(());
    }
    let mut payload: Value = serde_json::from_slice(&body).unwrap_or(json!({}));
    let rt = Runtime::open_default().ok();
    payload = match &rt {
        Some(rt) => rt.freeze_request(payload, capability),
        None => fallback_rewrite(payload, capability),
    };
    if dry_run || upstream.is_none() {
        let pretty = serde_json::to_vec_pretty(&json!({
            "ok": true,
            "dry_run": true,
            "path": path,
            "request": payload,
        }))?;
        write_http(&mut stream, 200, "application/json", &pretty).await?;
        return Ok(());
    }
    let headers = hop_headers(&head);
    let session = payload
        .get("ctx_session")
        .and_then(Value::as_str)
        .unwrap_or("intercept")
        .to_string();
    let cwd = payload
        .get("metadata")
        .and_then(|m| m.get("cwd"))
        .and_then(Value::as_str)
        .map(std::path::PathBuf::from);
    let target = upstream.unwrap();
    let mut loops = 0u32;
    loop {
        if let Some(obj) = payload.as_object_mut() {
            obj.remove("ctx_prefix_hash");
            obj.remove("ctx_session");
        }
        let raw = serde_json::to_vec(&payload)?;
        let forwarded = forward(target, &path, &raw, &headers).await?;
        if !capability || rt.is_none() || loops >= 8 {
            write_raw(&mut stream, &forwarded).await?;
            return Ok(());
        }
        let json_body = http_json_body(&forwarded);
        let Ok(resp) = serde_json::from_slice::<Value>(&json_body) else {
            write_raw(&mut stream, &forwarded).await?;
            return Ok(());
        };
        let uses = tool_uses(&resp);
        if uses.is_empty() {
            write_raw(&mut stream, &forwarded).await?;
            return Ok(());
        }
        let rt = rt.as_ref().unwrap();
        let mut results = Vec::new();
        for (id, name, input) in &uses {
            let out = rt.ctx_tool(&session, cwd.as_deref(), name, input);
            results.push((id.clone(), out));
        }
        append_tool_round(&mut payload, &resp, &results);
        loops += 1;
    }
}

fn fallback_rewrite(mut payload: Value, capability: bool) -> Value {
    if capability {
        if let Some(arr) = payload.get_mut("tools").and_then(|v| v.as_array_mut()) {
            *arr = ctx_core::wrap_tools(arr);
        }
    }
    payload
}

fn hop_headers(head: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for line in head.lines().skip(1) {
        let Some((k, v)) = line.split_once(':') else {
            continue;
        };
        let key = k.trim();
        let l = key.to_ascii_lowercase();
        if matches!(
            l.as_str(),
            "x-api-key"
                | "authorization"
                | "anthropic-version"
                | "anthropic-beta"
                | "openai-beta"
                | "content-type"
        ) {
            out.push((key.to_string(), v.trim().to_string()));
        }
    }
    out
}

fn tool_uses(resp: &Value) -> Vec<(String, String, Value)> {
    let mut out = Vec::new();
    if let Some(arr) = resp.get("content").and_then(|v| v.as_array()) {
        for block in arr {
            if block.get("type").and_then(Value::as_str) != Some("tool_use") {
                continue;
            }
            let name = block.get("name").and_then(Value::as_str).unwrap_or("");
            if !name.starts_with("ctx_") {
                continue;
            }
            let id = block.get("id").and_then(Value::as_str).unwrap_or("").to_string();
            let input = block.get("input").cloned().unwrap_or(json!({}));
            out.push((id, name.to_string(), input));
        }
    }
    if let Some(calls) = resp
        .pointer("/choices/0/message/tool_calls")
        .and_then(|v| v.as_array())
    {
        for call in calls {
            let name = call
                .pointer("/function/name")
                .and_then(Value::as_str)
                .unwrap_or("");
            if !name.starts_with("ctx_") {
                continue;
            }
            let id = call.get("id").and_then(Value::as_str).unwrap_or("").to_string();
            let args = call
                .pointer("/function/arguments")
                .and_then(Value::as_str)
                .and_then(|s| serde_json::from_str(s).ok())
                .unwrap_or(json!({}));
            out.push((id, name.to_string(), args));
        }
    }
    out
}

fn append_tool_round(payload: &mut Value, resp: &Value, results: &[(String, String)]) {
    let msgs = payload
        .get_mut("messages")
        .and_then(|v| v.as_array_mut());
    let Some(msgs) = msgs else {
        return;
    };
    if let Some(content) = resp.get("content") {
        msgs.push(json!({"role": "assistant", "content": content}));
        let blocks: Vec<Value> = results
            .iter()
            .map(|(id, body)| {
                json!({
                    "type": "tool_result",
                    "tool_use_id": id,
                    "content": body
                })
            })
            .collect();
        msgs.push(json!({"role": "user", "content": blocks}));
        return;
    }
    if let Some(msg) = resp.pointer("/choices/0/message") {
        msgs.push(msg.clone());
        for (id, body) in results {
            msgs.push(json!({"role": "tool", "tool_call_id": id, "content": body}));
        }
    }
}

async fn read_http(stream: &mut TcpStream) -> anyhow::Result<(String, Vec<u8>)> {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 8192];
    loop {
        let n = stream.read(&mut tmp).await?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&tmp[..n]);
        if let Some(i) = find_header_end(&buf) {
            let head = String::from_utf8_lossy(&buf[..i]).into_owned();
            let mut body = buf[i..].to_vec();
            let want = content_length(&head).unwrap_or(0);
            while body.len() < want {
                let n = stream.read(&mut tmp).await?;
                if n == 0 {
                    break;
                }
                body.extend_from_slice(&tmp[..n]);
            }
            if body.len() > want && want > 0 {
                body.truncate(want);
            }
            return Ok((head, body));
        }
        if buf.len() > 16 * 1024 * 1024 {
            break;
        }
    }
    Ok((String::from_utf8_lossy(&buf).into_owned(), Vec::new()))
}

fn find_header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|i| i + 4)
        .or_else(|| buf.windows(2).position(|w| w == b"\n\n").map(|i| i + 2))
}

fn content_length(head: &str) -> Option<usize> {
    for line in head.lines() {
        let Some((k, v)) = line.split_once(':') else {
            continue;
        };
        if k.eq_ignore_ascii_case("content-length") {
            return v.trim().parse().ok();
        }
    }
    None
}

fn request_line(head: &str) -> Option<(String, String)> {
    let line = head.lines().next()?;
    let mut parts = line.split_whitespace();
    Some((parts.next()?.to_string(), parts.next()?.to_string()))
}

async fn write_http(stream: &mut TcpStream, status: u16, ctype: &str, body: &[u8]) -> anyhow::Result<()> {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        _ => "Error",
    };
    let head = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes()).await?;
    stream.write_all(body).await?;
    stream.flush().await?;
    Ok(())
}

async fn write_raw(stream: &mut TcpStream, bytes: &[u8]) -> anyhow::Result<()> {
    stream.write_all(bytes).await?;
    stream.flush().await?;
    Ok(())
}

async fn forward(
    upstream: &str,
    path: &str,
    body: &[u8],
    headers: &[(String, String)],
) -> anyhow::Result<Vec<u8>> {
    let url = format!("{}{}", upstream.trim_end_matches('/'), path);
    let tmp = tempfile::Builder::new().prefix("ctx-proxy").tempfile()?;
    tmp.as_file().write_all(body)?;
    let path_buf = tmp.path().to_path_buf();
    let mut cmd = tokio::process::Command::new("curl");
    cmd.args(["-sS", "-D", "-", "--max-time", "120", "-X", "POST"]);
    let mut has_ct = false;
    for (k, v) in headers {
        if k.eq_ignore_ascii_case("content-type") {
            has_ct = true;
        }
        cmd.args(["-H", &format!("{k}: {v}")]);
    }
    if !has_ct {
        cmd.args(["-H", "Content-Type: application/json"]);
    }
    cmd.args(["--data-binary", &format!("@{}", path_buf.display()), &url]);
    let out = cmd.output().await;
    match out {
        Ok(o) if o.status.success() => Ok(o.stdout),
        Ok(o) => {
            let mut msg = o.stderr;
            if msg.is_empty() {
                msg = o.stdout;
            }
            Ok(http_wrap(502, &msg))
        }
        Err(err) => Ok(http_wrap(502, err.to_string().as_bytes())),
    }
}

fn http_json_body(raw: &[u8]) -> Vec<u8> {
    if let Some(i) = find_header_end(raw) {
        raw[i..].to_vec()
    } else {
        raw.to_vec()
    }
}

fn http_wrap(status: u16, body: &[u8]) -> Vec<u8> {
    let mut out = format!(
        "HTTP/1.1 {status} Error\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .into_bytes();
    out.extend_from_slice(body);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use ctx_core::{CtxPaths, Runtime, Store};

    fn rt() -> (tempfile::TempDir, Runtime) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(CtxPaths::from_root(dir.path().to_path_buf())).unwrap();
        (dir, Runtime::open(store))
    }

    #[test]
    fn freeze_sorts_tools_and_hashes_prefix() {
        let (_d, rt) = rt();
        let out = rt.freeze_request(
            json!({
                "model": "claude-sonnet-4",
                "system": "You are {ts: 2026-08-20T10:00:00Z}",
                "tools": [{"name": "b"}, {"name": "a"}]
            }),
            false,
        );
        let names: Vec<_> = out["tools"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|t| t.get("name").and_then(Value::as_str))
            .collect();
        assert_eq!(names, vec!["a", "b"]);
        assert!(out.get("ctx_prefix_hash").is_some());
        assert!(out["system"].as_array().unwrap()[0]["text"]
            .as_str()
            .unwrap()
            .contains("CTX protocol"));
    }

    #[test]
    fn capability_replaces_tool_list() {
        let (_d, rt) = rt();
        let out = rt.freeze_request(
            json!({"tools": [{"name": "github"}, {"name": "slack"}]}),
            true,
        );
        let names: Vec<_> = out["tools"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|t| t.get("name").and_then(Value::as_str))
            .collect();
        assert!(names.contains(&"ctx_exec"));
        assert_eq!(names.len(), 5);
    }

    #[test]
    fn extracts_anthropic_tool_use() {
        let resp = json!({
            "content": [
                {"type": "text", "text": "ok"},
                {"type": "tool_use", "id": "t1", "name": "ctx_search", "input": {"query": "401"}}
            ]
        });
        let uses = tool_uses(&resp);
        assert_eq!(uses.len(), 1);
        assert_eq!(uses[0].1, "ctx_search");
    }
}
