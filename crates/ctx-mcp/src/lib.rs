//! Minimal MCP stdio server using JSON-RPC Content-Length framing.

use std::io::{BufRead, BufReader, Write};

use serde_json::{json, Value};

use ctx_core::{format_why, Runtime, Snapshot, WorkingSet};

const PROTOCOL: &str = "2024-11-05";

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
    while let Some(msg) = read_message(&mut reader)? {
        if msg.get("method").and_then(|m| m.as_str()) == Some("notifications/initialized") {
            continue;
        }
        if let Some(resp) = handle(&runtime, &msg) {
            write_message(&mut stdout, &resp)?;
        }
    }
    Ok(())
}

fn read_message<R: BufRead>(reader: &mut R) -> std::io::Result<Option<Value>> {
    let mut content_length: Option<usize> = None;
    loop {
        let mut line = String::new();
        let n = reader.read_line(&mut line)?;
        if n == 0 {
            return Ok(None);
        }
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            break;
        }
        if let Some(rest) = trimmed.strip_prefix("Content-Length:") {
            content_length = rest.trim().parse().ok();
        }
    }
    let Some(len) = content_length else {
        return Ok(None);
    };
    let mut buf = vec![0u8; len];
    reader.read_exact(&mut buf)?;
    Ok(serde_json::from_slice(&buf).ok())
}

fn write_message<W: Write>(writer: &mut W, msg: &Value) -> std::io::Result<()> {
    let body = serde_json::to_vec(msg)?;
    write!(writer, "Content-Length: {}\r\n\r\n", body.len())?;
    writer.write_all(&body)?;
    writer.flush()
}

fn handle(runtime: &Runtime, msg: &Value) -> Option<Value> {
    let method = msg.get("method")?.as_str()?;
    let id = msg.get("id").cloned();
    let params = msg.get("params").cloned().unwrap_or(json!({}));
    let result = match method {
        "initialize" => json!({
            "protocolVersion": PROTOCOL,
            "capabilities": { "tools": {} },
            "serverInfo": { "name": "ctx", "version": env!("CARGO_PKG_VERSION") }
        }),
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
                Ok(ws) => ws.render(),
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
        let dir = tempfile::tempdir().unwrap();
        let store =
            ctx_store::Store::open(ctx_store::CtxPaths::from_root(dir.path().to_path_buf()))
                .unwrap();
        let rt = Runtime::open(store);
        let msg = call_tool(&rt, "ctx_fetch", &json!({}));
        assert!(msg.contains("uri is required"), "{msg}");
    }
}
