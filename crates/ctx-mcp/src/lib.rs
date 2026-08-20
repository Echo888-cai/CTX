//! Minimal MCP stdio server.
//!
//! Spec transport is newline-delimited JSON (no embedded newlines).
//! `Content-Length` framing is still *accepted* so a stale client can talk;
//! replies match the request's framing.

use std::io::{BufRead, BufReader, Write};

use serde_json::{json, Value};

use ctx_core::{format_why, render_spine, Runtime, Snapshot, WorkingSet};

const DEFAULT_PROTOCOL: &str = "2024-11-05";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Framing {
    Ndjson,
    Lsp,
}

pub fn serve(runtime: Runtime) -> std::io::Result<()> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(async move {
            tokio::task::spawn_blocking(move || serve_sync(runtime))
                .await
                .unwrap_or_else(|e| Err(std::io::Error::other(e.to_string())))
        })
}

fn serve_sync(runtime: Runtime) -> std::io::Result<()> {
    let stdin = std::io::stdin();
    let mut reader = BufReader::new(stdin.lock());
    let mut stdout = std::io::stdout();
    serve_io(&runtime, &mut reader, &mut stdout)
}

/// Drive the JSON-RPC loop on any reader/writer. Used by tests and doctor.
pub fn serve_io<R: BufRead, W: Write>(
    runtime: &Runtime,
    reader: &mut R,
    writer: &mut W,
) -> std::io::Result<()> {
    while let Some((msg, framing)) = read_message(reader)? {
        if msg.get("method").and_then(|m| m.as_str()) == Some("notifications/initialized") {
            continue;
        }
        if let Some(resp) = handle(runtime, &msg) {
            write_message(writer, framing, &resp)?;
        }
    }
    Ok(())
}

pub fn read_message<R: BufRead>(reader: &mut R) -> std::io::Result<Option<(Value, Framing)>> {
    let mut header = String::new();
    loop {
        header.clear();
        let n = reader.read_line(&mut header)?;
        if n == 0 {
            return Ok(None);
        }
        let trimmed = header.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            continue;
        }
        if let Some(rest) = content_length_value(trimmed) {
            // Consume remaining headers until the blank line, then `rest` bytes.
            loop {
                let mut line = String::new();
                let n = reader.read_line(&mut line)?;
                if n == 0 {
                    return Ok(None);
                }
                if line.trim_end_matches(['\r', '\n']).is_empty() {
                    break;
                }
            }
            let mut buf = vec![0u8; rest];
            reader.read_exact(&mut buf)?;
            let msg = serde_json::from_slice(&buf).map_err(std::io::Error::other)?;
            return Ok(Some((msg, Framing::Lsp)));
        }
        let msg = serde_json::from_str::<Value>(trimmed).map_err(std::io::Error::other)?;
        return Ok(Some((msg, Framing::Ndjson)));
    }
}

fn content_length_value(line: &str) -> Option<usize> {
    let (name, rest) = line.split_once(':')?;
    if !name.eq_ignore_ascii_case("content-length") {
        return None;
    }
    rest.trim().parse().ok()
}

pub fn write_message<W: Write>(
    writer: &mut W,
    framing: Framing,
    msg: &Value,
) -> std::io::Result<()> {
    let body = serde_json::to_vec(msg)?;
    match framing {
        Framing::Ndjson => {
            writer.write_all(&body)?;
            writer.write_all(b"\n")?;
        }
        Framing::Lsp => {
            write!(writer, "Content-Length: {}\r\n\r\n", body.len())?;
            writer.write_all(&body)?;
        }
    }
    writer.flush()
}

fn handle(runtime: &Runtime, msg: &Value) -> Option<Value> {
    let method = msg.get("method")?.as_str()?;
    let id = msg.get("id").cloned();
    let params = msg.get("params").cloned().unwrap_or(json!({}));
    let result = match method {
        "initialize" => {
            let protocol = params
                .get("protocolVersion")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or(DEFAULT_PROTOCOL);
            json!({
                "protocolVersion": protocol,
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "ctx", "version": env!("CARGO_PKG_VERSION") }
            })
        }
        "tools/list" => json!({ "tools": tools_list() }),
        "tools/call" => {
            let name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let args = params.get("arguments").cloned().unwrap_or(json!({}));
            let text = call_tool(runtime, name, &args);
            json!({
                "content": [{ "type": "text", "text": text }]
            })
        }
        "ping" => json!({}),
        _ => {
            if id.is_some() {
                return Some(json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": { "code": -32601, "message": format!("method not found: {method}") }
                }));
            }
            return None;
        }
    };
    Some(json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result
    }))
}

fn tools_list() -> Vec<Value> {
    vec![
        json!({
            "name": "ctx_fetch",
            "description": "Page in a ctx:// URI. uri#frame or query selects a region. query=* full page.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "uri": { "type": "string", "description": "ctx:// URI" },
                    "id": { "type": "string", "description": "Alias for uri" },
                    "query": { "type": "string", "description": "Region, or * for full page" }
                }
            }
        }),
        json!({
            "name": "ctx_read",
            "description": "Read a file. Large files return a symbol index. query=region, *=full.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "query": { "type": "string" }
                },
                "required": ["path"]
            }
        }),
        json!({
            "name": "ctx_search",
            "description": "Find stored pages/frames by test name or error. Returns ctx://#frame.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": { "type": "string" },
                    "limit": { "type": "integer" }
                },
                "required": ["query"]
            }
        }),
        json!({
            "name": "ctx_inspect",
            "description": "HOT / WARM / COLD working set and mapped page URIs. task ranks by overlap.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "session": { "type": "string" },
                    "task": { "type": "string", "description": "Rank mapped pages by task tokens" }
                }
            }
        }),
        json!({
            "name": "ctx_why",
            "description": "Today's avoided tokens by reason.",
            "inputSchema": { "type": "object", "properties": {} }
        }),
    ]
}

fn call_tool(runtime: &Runtime, name: &str, args: &Value) -> String {
    match name {
        "ctx_fetch" => {
            let uri = args
                .get("uri")
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if uri.is_empty() {
                return "ctx_fetch error: uri is required (ctx://…)".into();
            }
            let query = args.get("query").and_then(|v| v.as_str());
            match runtime.fetch(uri, query) {
                Ok(s) => s,
                Err(e) => format!("ctx_fetch error: {e}"),
            }
        }
        "ctx_read" => {
            let path = args.get("path").and_then(|v| v.as_str()).unwrap_or("");
            if path.is_empty() {
                return "ctx_read error: path is required".into();
            }
            let query = args.get("query").and_then(|v| v.as_str());
            match runtime.read_file(path, query) {
                Ok(s) => s,
                Err(e) => format!("ctx_read error: {e}"),
            }
        }
        "ctx_search" => {
            let query = args.get("query").and_then(|v| v.as_str()).unwrap_or("");
            let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(8) as usize;
            match runtime.search(query, limit) {
                Ok(s) => s,
                Err(e) => format!("ctx_search error: {e}"),
            }
        }
        "ctx_inspect" => {
            let session = args.get("session").and_then(|v| v.as_str());
            let extra = args
                .get("task")
                .and_then(|v| v.as_str())
                .map(ctx_core::parse_task)
                .unwrap_or_default();
            match WorkingSet::query(&runtime.store, session, &extra) {
                Ok(ws) => {
                    let mut out = ws.render();
                    let epoch = render_spine(&runtime.store, session);
                    if !epoch.is_empty() {
                        out.push('\n');
                        out.push_str(&epoch);
                    }
                    out
                }
                Err(e) => format!("ctx_inspect error: {e}"),
            }
        }
        "ctx_why" => match Snapshot::capture(&runtime.store) {
            Ok(snap) => format_why(&snap.reasons_today, snap.today.avoided, snap.pages),
            Err(e) => format!("ctx_why error: {e}"),
        },
        other => format!("unknown tool: {other}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn rt() -> (tempfile::TempDir, Runtime) {
        let dir = tempfile::tempdir().unwrap();
        let store =
            ctx_store::Store::open(ctx_store::CtxPaths::from_root(dir.path().to_path_buf()))
                .unwrap();
        (dir, Runtime::open(store))
    }

    fn roundtrip(runtime: &Runtime, request: &str) -> (String, Framing) {
        let mut input = Cursor::new(request.as_bytes().to_vec());
        let mut output = Vec::new();
        serve_io(runtime, &mut input, &mut output).unwrap();
        let framing = if output.starts_with(b"Content-Length:") {
            Framing::Lsp
        } else {
            Framing::Ndjson
        };
        (String::from_utf8(output).unwrap(), framing)
    }

    #[test]
    fn tools_include_search_and_fetch_uri() {
        let tools = tools_list();
        let names: Vec<_> = tools
            .iter()
            .filter_map(|t| t.get("name").and_then(|n| n.as_str()))
            .collect();
        assert!(names.contains(&"ctx_fetch"));
        assert!(names.contains(&"ctx_search"));
        assert!(names.contains(&"ctx_inspect"));
        assert!(names.contains(&"ctx_why"));
        assert!(names.contains(&"ctx_read"));
        let fetch = tools.iter().find(|t| t["name"] == "ctx_fetch").unwrap();
        assert!(fetch["inputSchema"]["properties"].get("uri").is_some());
        assert!(fetch["description"].as_str().unwrap().contains("query"));
    }

    #[test]
    fn fetch_requires_uri() {
        let (_dir, rt) = rt();
        let msg = call_tool(&rt, "ctx_fetch", &json!({}));
        assert!(msg.contains("uri is required"), "{msg}");
    }

    #[test]
    fn ndjson_initialize_echoes_client_protocol() {
        let (_dir, rt) = rt();
        let req = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"1"}}}
"#;
        let (out, framing) = roundtrip(&rt, req);
        assert_eq!(framing, Framing::Ndjson, "{out}");
        assert!(!out.contains("Content-Length"), "{out}");
        let msg: Value = serde_json::from_str(out.trim_end()).unwrap();
        assert_eq!(
            msg["result"]["protocolVersion"].as_str(),
            Some("2025-06-18"),
            "{msg}"
        );
    }

    #[test]
    fn ndjson_tools_list_includes_page_fault_tools() {
        let (_dir, rt) = rt();
        let req = r#"{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}
"#;
        let (out, framing) = roundtrip(&rt, req);
        assert_eq!(framing, Framing::Ndjson);
        let msg: Value = serde_json::from_str(out.trim_end()).unwrap();
        let names: Vec<_> = msg["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|t| t.get("name").and_then(|n| n.as_str()))
            .collect();
        assert!(names.contains(&"ctx_read"), "{names:?}");
        assert!(names.contains(&"ctx_fetch"), "{names:?}");
    }

    #[test]
    fn lsp_framing_still_accepted() {
        let (_dir, rt) = rt();
        let body = br#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05"}}"#;
        let req = format!(
            "Content-Length: {}\r\n\r\n{}",
            body.len(),
            std::str::from_utf8(body).unwrap()
        );
        let (out, framing) = roundtrip(&rt, &req);
        assert_eq!(framing, Framing::Lsp, "{out}");
        assert!(out.starts_with("Content-Length:"), "{out}");
        let json = out.split("\r\n\r\n").nth(1).unwrap();
        let msg: Value = serde_json::from_str(json).unwrap();
        assert_eq!(
            msg["result"]["protocolVersion"].as_str(),
            Some("2024-11-05"),
            "{msg}"
        );
    }

    #[test]
    fn initialize_without_protocol_uses_default() {
        let (_dir, rt) = rt();
        let req = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}
"#;
        let (out, _) = roundtrip(&rt, req);
        let msg: Value = serde_json::from_str(out.trim_end()).unwrap();
        assert_eq!(
            msg["result"]["protocolVersion"].as_str(),
            Some(DEFAULT_PROTOCOL)
        );
    }
}
