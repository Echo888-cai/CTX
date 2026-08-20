//! Intercept-plane freeze + Capability FS execution.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{json, Value};

use ctx_pager::WorkingSet;
use ctx_protocol::{CtxEvent, Harness, ToolRef};
use ctx_store::blake3_hex;

use crate::canonical::canonicalize_json;
use crate::capability::{tools as frozen_tools, wrap_tools};
use crate::runtime::Runtime;
use crate::spine::Spine;

impl Runtime {
    /// Canonicalize, index foreign tools, freeze L0/L1 into `system`.
    pub fn freeze_request(&self, mut payload: Value, capability: bool) -> Value {
        let session = session_of(&payload);
        let cwd = cwd_of(&payload);
        let model = payload
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let _ = self.store.ensure_session_with_model(
            &session,
            "intercept",
            cwd.as_deref().map(|p| p.to_str().unwrap_or("")),
            if model.is_empty() { None } else { Some(model.as_str()) },
        );
        let (snap, n, manifest) = crate::overlay::capture(cwd.as_deref());
        let _ = self.store.put_workspace_snapshot(&snap, n, &manifest);
        let tools_h = crate::capability::tools_hash();
        let prefix = crate::canonical::prefix_hash(&[
            crate::capability::protocol_text(),
            &snap,
            &tools_h,
        ]);
        let epoch = self
            .store
            .ensure_epoch(
                &session,
                &model,
                "",
                &tools_h,
                crate::capability::PROTOCOL_VERSION,
                &snap,
                &prefix,
            )
            .ok();
        let overlays = epoch
            .as_ref()
            .and_then(|e| self.store.overlays_for(&session, e.epoch).ok())
            .unwrap_or_default();
        let journal = epoch
            .as_ref()
            .and_then(|e| self.store.journal_text(&session, e.epoch).ok())
            .unwrap_or_default();
        let ws = WorkingSet::query(&self.store, Some(&session), &[])
            .ok()
            .map(|w| w.render_mapped())
            .unwrap_or_default();
        let snap_id = epoch
            .as_ref()
            .map(|e| e.workspace_snapshot.clone())
            .unwrap_or(snap);
        let spine = Spine::assemble(&snap_id, &overlays, &journal, &ws, "");

        if let Some(obj) = payload.as_object_mut() {
            if let Some(tools) = obj.get_mut("tools") {
                if let Some(arr) = tools.as_array_mut() {
                    if capability {
                        index_tools(&self.store, arr);
                        *arr = wrap_tools(arr);
                    } else {
                        *arr = arr.iter().map(canonicalize_json).collect();
                        arr.sort_by(|a, b| {
                            a.get("name")
                                .and_then(Value::as_str)
                                .unwrap_or("")
                                .cmp(b.get("name").and_then(Value::as_str).unwrap_or(""))
                        });
                    }
                }
            }
            if let Some(msgs) = obj.get_mut("messages").and_then(|v| v.as_array_mut()) {
                for m in msgs.iter_mut() {
                    *m = canonicalize_json(m);
                }
            }
        }

        let original_system = payload.get("system").cloned().unwrap_or(Value::Null);
        if looks_anthropic(&payload) || payload.get("system").is_some() {
            payload["system"] = spine.freeze_system(&original_system);
        } else if let Some(msgs) = payload.get_mut("messages").and_then(|v| v.as_array_mut()) {
            msgs.insert(
                0,
                json!({"role": "system", "content": spine.prefix()}),
            );
        } else {
            payload["system"] = spine.freeze_system(&original_system);
        }
        if let Some(obj) = payload.as_object_mut() {
            obj.insert("ctx_prefix_hash".into(), json!(spine.prefix_hash));
            obj.insert("ctx_session".into(), json!(session));
        }
        canonicalize_json(&payload)
    }

    /// Run one frozen CTX tool. Used by the intercept loop and tests.
    pub fn ctx_tool(&self, session: &str, cwd: Option<&Path>, name: &str, input: &Value) -> String {
        match name {
            "ctx_search" => {
                let q = input.get("query").and_then(Value::as_str).unwrap_or("");
                let limit = input.get("limit").and_then(Value::as_u64).unwrap_or(8) as usize;
                let mut out = String::new();
                if let Ok(caps) = self.store.search_capabilities(q, 8) {
                    for (handle, name, desc) in caps {
                        out.push_str(&format!("{handle}  {name}  {desc}\n"));
                    }
                }
                match self.search(q, limit) {
                    Ok(s) => {
                        out.push_str(&s);
                        out
                    }
                    Err(e) => format!("ctx_search error: {e}"),
                }
            }
            "ctx_fetch" => {
                let uri = input
                    .get("uri")
                    .or_else(|| input.get("id"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let query = input.get("query").and_then(Value::as_str);
                if uri.starts_with("capability://") {
                    return fetch_capability(&self.store, uri);
                }
                match self.fetch(uri, query) {
                    Ok(s) => s,
                    Err(e) => format!("ctx_fetch error: {e}"),
                }
            }
            "ctx_inspect" => {
                let sid = input
                    .get("session")
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                    .unwrap_or(session);
                let extra = input
                    .get("task")
                    .and_then(Value::as_str)
                    .map(crate::parse_task)
                    .unwrap_or_default();
                match WorkingSet::query(&self.store, Some(sid), &extra) {
                    Ok(ws) => {
                        let mut out = ws.render();
                        let epoch = crate::render_spine(&self.store, Some(sid));
                        if !epoch.is_empty() {
                            out.push('\n');
                            out.push_str(&epoch);
                        }
                        out
                    }
                    Err(e) => format!("ctx_inspect error: {e}"),
                }
            }
            "ctx_apply" => apply_overlay(self, session, cwd, input),
            "ctx_exec" => exec_handle(self, session, cwd, input),
            other => format!("unknown tool {other}"),
        }
    }
}

fn looks_anthropic(payload: &Value) -> bool {
    payload
        .get("tools")
        .and_then(|t| t.as_array())
        .and_then(|a| a.first())
        .map(|t| t.get("input_schema").is_some() || t.get("name") == Some(&json!("ctx_search")))
        .unwrap_or(false)
        || payload.get("system").is_some()
}

fn session_of(payload: &Value) -> String {
    ["ctx_session", "session"]
        .into_iter()
        .find_map(|k| payload.get(k).and_then(Value::as_str))
        .or_else(|| {
            payload
                .get("metadata")
                .and_then(|m| m.get("user_id").or_else(|| m.get("session_id")))
                .and_then(Value::as_str)
        })
        .filter(|s| !s.is_empty())
        .unwrap_or("intercept")
        .to_string()
}

fn cwd_of(payload: &Value) -> Option<PathBuf> {
    payload
        .get("metadata")
        .and_then(|m| m.get("cwd"))
        .and_then(Value::as_str)
        .map(PathBuf::from)
        .filter(|p| p.is_dir())
}

fn index_tools(store: &ctx_store::Store, tools: &[Value]) {
    for t in tools {
        let name = t.get("name").and_then(Value::as_str).unwrap_or("");
        if name.is_empty() || name.starts_with("ctx_") {
            continue;
        }
        let handle = format!("capability://{name}");
        let desc = t
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let schema = serde_json::to_string(t).unwrap_or_else(|_| "{}".into());
        let _ = store.put_capability(&handle, name, &desc, &schema);
    }
}

fn fetch_capability(store: &ctx_store::Store, uri: &str) -> String {
    match store.get_capability(uri) {
        Ok(Some((name, desc, schema))) => format!("{uri}\n{name}\n{desc}\n{schema}\n"),
        Ok(None) => format!("unknown capability {uri}\n"),
        Err(e) => format!("capability error: {e}"),
    }
}

fn apply_overlay(rt: &Runtime, session: &str, cwd: Option<&Path>, input: &Value) -> String {
    let path = input.get("path").and_then(Value::as_str).unwrap_or("");
    if path.is_empty() {
        return "ctx_apply error: path is required".into();
    }
    let contents = input
        .get("contents")
        .or_else(|| input.get("content"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let full = match cwd {
        Some(dir) if !Path::new(path).is_absolute() => dir.join(path),
        _ => PathBuf::from(path),
    };
    if let Some(parent) = full.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Err(e) = std::fs::write(&full, contents) {
        return format!("ctx_apply error: {e}");
    }
    let hash = blake3_hex(contents.as_bytes());
    let _ = rt.store.put_bytes(contents.as_bytes());
    let prev = rt
        .store
        .get_file_read(path)
        .ok()
        .flatten()
        .map(|r| r.content_hash)
        .unwrap_or_default();
    if prev != hash {
        let _ = rt.store.push_overlay(session, path, &prev, &hash);
        let _ = rt.store.push_journal(session, "overlay", path);
    }
    let _ = rt.store.upsert_file_read(&ctx_store::FileReadRecord {
        path: path.to_string(),
        content_hash: hash.clone(),
        last_uri: None,
        last_tokens: crate::estimate_tokens(contents),
        regions: json!([]),
        chunks: json!([]),
    });
    format!("applied {path}  {hash}\n")
}

fn exec_handle(rt: &Runtime, session: &str, cwd: Option<&Path>, input: &Value) -> String {
    let handle = input.get("handle").and_then(Value::as_str).unwrap_or("");
    let args = input.get("arguments").cloned().unwrap_or(json!({}));
    let name = handle
        .strip_prefix("capability://")
        .unwrap_or(handle)
        .trim();
    if name.is_empty() {
        return "ctx_exec error: handle is required".into();
    }
    if frozen_tools()
        .iter()
        .any(|t| t.get("name").and_then(Value::as_str) == Some(name))
    {
        return rt.ctx_tool(session, cwd, name, &args);
    }
    let lname = name.to_ascii_lowercase();
    if matches!(
        lname.as_str(),
        "shell" | "bash" | "sh" | "zsh" | "cmd" | "terminal"
    ) {
        let cmd = args
            .get("command")
            .or_else(|| args.get("cmd"))
            .and_then(Value::as_str)
            .unwrap_or("");
        return run_shell(rt, session, cwd, cmd);
    }
    if let Ok(Some((stored, desc, schema))) = rt
        .store
        .get_capability(&format!("capability://{name}"))
        .or_else(|_| rt.store.get_capability(handle))
    {
        let sl = stored.to_ascii_lowercase();
        if sl.contains("bash") || sl.contains("shell") {
            let cmd = args
                .get("command")
                .and_then(Value::as_str)
                .unwrap_or("");
            return run_shell(rt, session, cwd, cmd);
        }
        return format!(
            "capability {handle} is indexed ({stored}: {desc}) but not bound in this intercept.\n{schema}\n"
        );
    }
    format!("unknown capability {handle}\n")
}

fn run_shell(rt: &Runtime, session: &str, cwd: Option<&Path>, cmd: &str) -> String {
    if cmd.trim().is_empty() {
        return "ctx_exec error: command is required".into();
    }
    let mut command = Command::new("sh");
    command.arg("-c").arg(cmd);
    if let Some(dir) = cwd {
        command.current_dir(dir);
    }
    let out = command.output();
    let (code, body) = match out {
        Ok(o) => {
            let mut s = String::from_utf8_lossy(&o.stdout).into_owned();
            if !o.stderr.is_empty() {
                if !s.is_empty() {
                    s.push('\n');
                }
                s.push_str(&String::from_utf8_lossy(&o.stderr));
            }
            (o.status.code().unwrap_or(1), s)
        }
        Err(e) => return format!("ctx_exec error: {e}"),
    };
    let clipped = if body.len() > 64 * 1024 {
        format!("{}\n… truncated\n", &body[..64 * 1024])
    } else {
        body
    };
    let mut event = CtxEvent::tool_output(
        session,
        Harness::Unknown,
        ToolRef::new("Bash"),
        clipped,
    );
    event.metadata = json!({ "cwd": cwd.map(|p| p.display().to_string()), "exit_code": code });
    match rt.ingest(event) {
        Ok(r) => r.delivered,
        Err(e) => format!("ctx_exec ingest error: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ctx_store::{CtxPaths, Store};

    fn rt() -> (tempfile::TempDir, Runtime) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(CtxPaths::from_root(dir.path().to_path_buf())).unwrap();
        (dir, Runtime::open(store))
    }

    #[test]
    fn freeze_injects_protocol_and_breakpoint() {
        let (_d, rt) = rt();
        let out = rt.freeze_request(
            json!({
                "model": "claude-sonnet-4",
                "system": "You are a bot 2026-08-20T10:00:00Z",
                "tools": [{"name": "b"}, {"name": "a"}]
            }),
            false,
        );
        let sys = out["system"].as_array().unwrap();
        assert!(sys[0]["text"].as_str().unwrap().contains("CTX protocol"));
        assert_eq!(sys[1]["cache_control"]["type"], "ephemeral");
        assert!(out.get("ctx_prefix_hash").is_some());
        let names: Vec<_> = out["tools"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|t| t.get("name").and_then(Value::as_str))
            .collect();
        assert_eq!(names, vec!["a", "b"]);
    }

    #[test]
    fn capability_indexes_and_replaces() {
        let (_d, rt) = rt();
        let out = rt.freeze_request(
            json!({"tools": [{"name": "github", "description": "PRs"}]}),
            true,
        );
        let names: Vec<_> = out["tools"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|t| t.get("name").and_then(Value::as_str))
            .collect();
        assert_eq!(names.len(), 5);
        assert!(names.contains(&"ctx_exec"));
        let fetched = rt.ctx_tool(
            "intercept",
            None,
            "ctx_fetch",
            &json!({"uri": "capability://github"}),
        );
        assert!(fetched.contains("github"), "{fetched}");
    }

    #[test]
    fn apply_writes_and_overlays() {
        let (dir, rt) = rt();
        let cwd = dir.path();
        rt.store
            .ensure_epoch("s", "m", "", "", "", "snap", "p")
            .unwrap();
        let out = rt.ctx_tool(
            "s",
            Some(cwd),
            "ctx_apply",
            &json!({"path": "src/a.rs", "contents": "fn a() {}"}),
        );
        assert!(out.contains("applied"), "{out}");
        assert!(cwd.join("src/a.rs").is_file());
        let rows = rt.store.overlays_for("s", 1).unwrap();
        assert_eq!(rows.len(), 1);
    }
}
