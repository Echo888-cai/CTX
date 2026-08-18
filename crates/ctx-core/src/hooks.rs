use serde_json::{json, Value};

use ctx_optimizer::estimate_tokens;
use ctx_protocol::{CtxEvent, EventKind, Harness, ToolKind, ToolRef};

use crate::runtime::Runtime;
use crate::wrap::{is_already_wrapped, rewrite_shell_command};

#[derive(Debug, Clone)]
pub struct HookResponse {
    pub stdout: String,
    pub deny: bool,
}

/// Fail-open hook entry: never break the harness on CTX errors.
pub fn handle_hook(runtime: &Runtime, stdin_json: &str) -> HookResponse {
    let Ok(value) = serde_json::from_str::<Value>(stdin_json) else {
        return HookResponse {
            stdout: String::new(),
            deny: false,
        };
    };
    match handle_hook_inner(runtime, &value) {
        Ok(resp) => resp,
        Err(err) => {
            tracing::warn!(error = %err, "ctx hook failed; passing through (fail-open)");
            HookResponse {
                stdout: String::new(),
                deny: false,
            }
        }
    }
}

fn handle_hook_inner(runtime: &Runtime, value: &Value) -> anyhow::Result<HookResponse> {
    let event_name = value
        .get("hook_event_name")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let harness = detect_harness(value);

    match event_name {
        "PreToolUse" | "preToolUse" => pre_tool_use(runtime, value, harness),
        "PostToolUse" | "postToolUse" | "PostToolUseFailure" | "postToolUseFailure" => {
            post_tool_use(runtime, value, harness, event_name)
        }
        "beforeReadFile" => before_read_file(runtime, value, harness),
        "SessionStart" | "sessionStart" => session_start(runtime, value, harness),
        "SessionEnd" | "sessionEnd" | "stop" => session_end(runtime, value, harness),
        "UserPromptSubmit" | "beforeSubmitPrompt" => prompt_submit(runtime, value, harness),
        "PreCompact" | "preCompact" => compact_hook(runtime, value, harness, true),
        "PostCompact" | "postCompact" => compact_hook(runtime, value, harness, false),
        "afterShellExecution" => after_shell(runtime, value, harness),
        "afterMCPExecution" => after_mcp(runtime, value, harness),
        "beforeShellExecution" => Ok(HookResponse {
            stdout: json!({ "permission": "allow" }).to_string(),
            deny: false,
        }),
        _ => Ok(HookResponse {
            stdout: String::new(),
            deny: false,
        }),
    }
}

fn detect_harness(value: &Value) -> Harness {
    if value.get("cursor_version").is_some() || value.get("conversation_id").is_some() {
        return Harness::Cursor;
    }
    if value.get("session_id").is_some() || value.get("transcript_path").is_some() {
        return Harness::ClaudeCode;
    }
    Harness::Unknown
}

fn session_id(value: &Value) -> String {
    value
        .get("session_id")
        .or_else(|| value.get("conversation_id"))
        .and_then(|v| v.as_str())
        .unwrap_or("default")
        .to_string()
}

fn pre_tool_use(
    runtime: &Runtime,
    value: &Value,
    harness: Harness,
) -> anyhow::Result<HookResponse> {
    let tool_name = value
        .get("tool_name")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let tool_input = value.get("tool_input").cloned().unwrap_or(json!({}));
    let kind = ToolKind::from_tool_name(tool_name);

    if kind == ToolKind::Shell {
        if let Some(updated) = rewrite_shell_command(&tool_input) {
            let body = match harness {
                Harness::ClaudeCode => json!({
                    "hookSpecificOutput": {
                        "hookEventName": "PreToolUse",
                        "permissionDecision": "allow",
                        "updatedInput": updated
                    }
                }),
                _ => json!({
                    "permission": "allow",
                    "updated_input": updated
                }),
            };
            return Ok(HookResponse {
                stdout: body.to_string(),
                deny: false,
            });
        }
    }

    if kind == ToolKind::File && harness == Harness::Cursor {
        if let Some(path) = tool_input
            .get("path")
            .or_else(|| tool_input.get("file_path"))
            .and_then(|v| v.as_str())
        {
            if let Ok(meta) = std::fs::metadata(path) {
                let approx = (meta.len() as u32) / 3;
                if approx > runtime.config.large_file_tokens {
                    let msg = format!("large file {path}. ctx_read(\"{path}\", query=\"…\")");
                    return Ok(HookResponse {
                        stdout: json!({
                            "permission": "deny",
                            "agent_message": msg
                        })
                        .to_string(),
                        deny: true,
                    });
                }
            }
        }
    }

    let empty = match harness {
        Harness::ClaudeCode => json!({
            "hookSpecificOutput": {
                "hookEventName": "PreToolUse",
                "permissionDecision": "allow"
            }
        }),
        _ => json!({ "permission": "allow" }),
    };
    Ok(HookResponse {
        stdout: empty.to_string(),
        deny: false,
    })
}

fn post_tool_use(
    runtime: &Runtime,
    value: &Value,
    harness: Harness,
    event_name: &str,
) -> anyhow::Result<HookResponse> {
    let tool_name = value
        .get("tool_name")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let payload = extract_payload(value);
    if extract_command(value).is_some_and(|c| is_already_wrapped(&c)) {
        // `ctx exec` already stored the raw page and printed the working set.
        return Ok(HookResponse {
            stdout: String::new(),
            deny: false,
        });
    }
    let mut metadata = json!({
        "cwd": value.get("cwd"),
    });
    if let Some(path) = extract_path(value) {
        insert_meta(&mut metadata, "path", path);
    }
    if let Some(code) = extract_exit_code(value, event_name) {
        insert_meta(&mut metadata, "exit_code", code);
    }
    if let Some(cmd) = extract_command(value) {
        insert_meta(&mut metadata, "command", cmd);
    }

    let event = CtxEvent {
        event: EventKind::from_hook_name(event_name),
        session: session_id(value),
        harness,
        tool: Some(ToolRef::new(tool_name)),
        payload,
        task_context: None,
        metadata,
    };
    let result = runtime.ingest(event)?;
    if !result.replaced {
        return Ok(HookResponse {
            stdout: String::new(),
            deny: false,
        });
    }

    let stdout = match harness {
        Harness::ClaudeCode => claude_updated_output(tool_name, value, &result.delivered),
        Harness::Cursor => {
            if ToolKind::from_tool_name(tool_name) == ToolKind::Mcp
                || tool_name.to_ascii_lowercase().starts_with("mcp")
            {
                cursor_post_output(&result.delivered)
            } else {
                // Native shell/file output is rewritten via `ctx exec` / MCP. Do not
                // append additional_context — that would add tokens, not remove them.
                String::new()
            }
        }
        Harness::Unknown => claude_updated_output(tool_name, value, &result.delivered),
    };
    Ok(HookResponse {
        stdout,
        deny: false,
    })
}

fn claude_updated_output(tool_name: &str, original: &Value, delivered: &str) -> String {
    let kind = ToolKind::from_tool_name(tool_name);
    let updated = if kind == ToolKind::Shell {
        let mut obj = original
            .get("tool_response")
            .cloned()
            .unwrap_or_else(|| json!({}));
        if obj.is_object() {
            if let Some(map) = obj.as_object_mut() {
                map.insert("stdout".into(), delivered.to_string().into());
                map.entry("stderr").or_insert_with(|| json!(""));
                map.entry("interrupted").or_insert_with(|| json!(false));
                map.entry("isImage").or_insert_with(|| json!(false));
            }
            obj
        } else {
            json!({
                "stdout": delivered,
                "stderr": "",
                "interrupted": false,
                "isImage": false
            })
        }
    } else {
        json!(delivered)
    };
    json!({
        "hookSpecificOutput": {
            "hookEventName": "PostToolUse",
            "updatedToolOutput": updated
        }
    })
    .to_string()
}

fn cursor_post_output(delivered: &str) -> String {
    json!({
        "updated_mcp_tool_output": { "content": delivered }
    })
    .to_string()
}

fn before_read_file(
    runtime: &Runtime,
    value: &Value,
    harness: Harness,
) -> anyhow::Result<HookResponse> {
    let path = value
        .get("file_path")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let content = value.get("content").and_then(|v| v.as_str()).unwrap_or("");
    let tokens = estimate_tokens(content);
    if !content.is_empty() {
        let event = CtxEvent {
            event: EventKind::FileRead,
            session: session_id(value),
            harness,
            tool: Some(ToolRef::new("Read")),
            payload: content.to_string(),
            task_context: None,
            metadata: json!({ "path": path, "cwd": value.get("cwd") }),
        };
        let _ = runtime.ingest(event);
    }
    if tokens > runtime.config.large_file_tokens && harness == Harness::Cursor {
        return Ok(HookResponse {
            stdout: json!({
                "permission": "deny",
                "user_message": format!("large file {path} → ctx_read")
            })
            .to_string(),
            deny: true,
        });
    }
    Ok(HookResponse {
        stdout: json!({ "permission": "allow" }).to_string(),
        deny: false,
    })
}

fn after_shell(runtime: &Runtime, value: &Value, harness: Harness) -> anyhow::Result<HookResponse> {
    if value
        .get("command")
        .and_then(|v| v.as_str())
        .is_some_and(is_already_wrapped)
    {
        return Ok(HookResponse {
            stdout: String::new(),
            deny: false,
        });
    }
    let payload = value
        .get("output")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let event = CtxEvent {
        event: EventKind::ToolOutput,
        session: session_id(value),
        harness,
        tool: Some(ToolRef::new("Shell")),
        payload,
        task_context: None,
        metadata: json!({
            "cwd": value.get("cwd"),
            "command": value.get("command"),
            "exit_code": extract_exit_code(value, "afterShellExecution"),
        }),
    };
    let _ = runtime.ingest(event)?;
    Ok(HookResponse {
        stdout: String::new(),
        deny: false,
    })
}

fn after_mcp(runtime: &Runtime, value: &Value, harness: Harness) -> anyhow::Result<HookResponse> {
    let tool_name = value
        .get("tool_name")
        .and_then(|v| v.as_str())
        .unwrap_or("mcp");
    let payload = value
        .get("result_json")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let event = CtxEvent {
        event: EventKind::ToolOutput,
        session: session_id(value),
        harness,
        tool: Some(ToolRef {
            kind: ToolKind::Mcp,
            name: tool_name.to_string(),
        }),
        payload,
        task_context: None,
        metadata: json!({}),
    };
    let result = runtime.ingest(event)?;
    if result.replaced {
        return Ok(HookResponse {
            stdout: json!({
                "updated_mcp_tool_output": { "content": result.delivered }
            })
            .to_string(),
            deny: false,
        });
    }
    Ok(HookResponse {
        stdout: String::new(),
        deny: false,
    })
}

fn prompt_submit(
    runtime: &Runtime,
    value: &Value,
    harness: Harness,
) -> anyhow::Result<HookResponse> {
    let prompt = extract_prompt(value);
    let event = CtxEvent {
        event: EventKind::PromptSubmit,
        session: session_id(value),
        harness,
        tool: None,
        payload: prompt,
        task_context: None,
        metadata: json!({ "cwd": value.get("cwd") }),
    };
    let result = runtime.ingest(event)?;
    if result.delivered.is_empty() {
        return Ok(HookResponse {
            stdout: String::new(),
            deny: false,
        });
    }
    let stdout = match harness {
        Harness::ClaudeCode => json!({
            "hookSpecificOutput": {
                "hookEventName": "UserPromptSubmit",
                "additionalContext": result.delivered
            }
        }),
        _ => json!({ "additional_context": result.delivered }),
    };
    Ok(HookResponse {
        stdout: stdout.to_string(),
        deny: false,
    })
}

fn compact_hook(
    runtime: &Runtime,
    value: &Value,
    harness: Harness,
    keep: bool,
) -> anyhow::Result<HookResponse> {
    let event = CtxEvent {
        event: EventKind::Compact,
        session: session_id(value),
        harness,
        tool: None,
        payload: String::new(),
        task_context: None,
        metadata: json!({
            "cwd": value.get("cwd"),
            "keep": keep,
        }),
    };
    let result = runtime.ingest(event)?;
    // PreCompact: plain text (must not start with `{`) so Claude can keep
    // the page table. hookSpecificOutput is rejected for this event.
    Ok(HookResponse {
        stdout: result.delivered,
        deny: false,
    })
}

fn session_start(
    runtime: &Runtime,
    value: &Value,
    harness: Harness,
) -> anyhow::Result<HookResponse> {
    let mut metadata = json!({ "cwd": value.get("cwd") });
    if let Some(model) = hook_model(value) {
        insert_meta(&mut metadata, "model", model);
    }
    let event = CtxEvent {
        event: EventKind::SessionStart,
        session: session_id(value),
        harness,
        tool: None,
        payload: String::new(),
        task_context: None,
        metadata,
    };
    let result = runtime.ingest(event)?;
    let stdout = match harness {
        Harness::ClaudeCode => json!({
            "hookSpecificOutput": {
                "hookEventName": "SessionStart",
                "additionalContext": result.delivered
            }
        }),
        _ => json!({ "additional_context": result.delivered }),
    };
    Ok(HookResponse {
        stdout: stdout.to_string(),
        deny: false,
    })
}

fn hook_model(value: &Value) -> Option<&str> {
    ["model", "model_id", "modelName"]
        .into_iter()
        .find_map(|key| value.get(key).and_then(Value::as_str))
        .map(str::trim)
        .filter(|model| !model.is_empty())
}

fn session_end(runtime: &Runtime, value: &Value, harness: Harness) -> anyhow::Result<HookResponse> {
    let event = CtxEvent {
        event: EventKind::SessionEnd,
        session: session_id(value),
        harness,
        tool: None,
        payload: String::new(),
        task_context: None,
        metadata: json!({}),
    };
    let _ = runtime.ingest(event)?;
    Ok(HookResponse {
        stdout: String::new(),
        deny: false,
    })
}

fn insert_meta(metadata: &mut Value, key: &str, value: impl Into<Value>) {
    if let Some(obj) = metadata.as_object_mut() {
        obj.insert(key.into(), value.into());
    }
}

fn extract_payload(value: &Value) -> String {
    if let Some(s) = value.get("tool_output").and_then(|v| v.as_str()) {
        return s.to_string();
    }
    if let Some(err) = value.get("error").and_then(|v| v.as_str()) {
        return err.to_string();
    }
    match value.get("tool_response") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Object(map)) => {
            if let Some(Value::String(s)) = map.get("stdout") {
                let mut out = s.clone();
                if let Some(Value::String(err)) = map.get("stderr") {
                    if !err.is_empty() {
                        out.push('\n');
                        out.push_str(err);
                    }
                }
                return out;
            }
            if let Some(s) = map.get("content").and_then(|v| v.as_str()) {
                return s.to_string();
            }
            if let Some(s) = map
                .get("file")
                .and_then(|f| f.get("content"))
                .and_then(|v| v.as_str())
            {
                return s.to_string();
            }
            serde_json::to_string_pretty(&Value::Object(map.clone())).unwrap_or_default()
        }
        Some(other) => other.to_string(),
        None => String::new(),
    }
}

fn extract_prompt(value: &Value) -> String {
    value
        .get("prompt")
        .or_else(|| value.get("text"))
        .or_else(|| value.get("content"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}

fn extract_path(value: &Value) -> Option<String> {
    value
        .get("tool_input")
        .and_then(|t| t.get("file_path").or_else(|| t.get("path")))
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .or_else(|| {
            value
                .get("tool_response")
                .and_then(|t| t.get("file"))
                .and_then(|f| f.get("filePath").or_else(|| f.get("file_path")))
                .and_then(|v| v.as_str())
                .map(str::to_string)
        })
        .or_else(|| {
            value
                .get("file_path")
                .or_else(|| value.get("path"))
                .and_then(|v| v.as_str())
                .map(str::to_string)
        })
}

fn extract_command(value: &Value) -> Option<String> {
    value
        .get("tool_input")
        .and_then(|t| t.get("command"))
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .or_else(|| {
            value
                .get("command")
                .and_then(|v| v.as_str())
                .map(str::to_string)
        })
}

fn extract_exit_code(value: &Value, event_name: &str) -> Option<i64> {
    for key in ["exit_code", "exitCode", "exit"] {
        if let Some(n) = value.get(key).and_then(ctx_protocol::json_i64) {
            return Some(n);
        }
    }
    if let Some(resp) = value.get("tool_response") {
        for key in ["exit_code", "exitCode", "exit"] {
            if let Some(n) = resp.get(key).and_then(ctx_protocol::json_i64) {
                return Some(n);
            }
        }
    }
    if event_name.contains("Failure") || event_name.contains("failure") {
        return Some(1);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use ctx_store::{CtxPaths, Store};

    fn rt() -> (tempfile::TempDir, Runtime) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(CtxPaths::from_root(dir.path().to_path_buf())).unwrap();
        let runtime = Runtime::open(store);
        (dir, runtime)
    }

    #[test]
    fn session_end_does_not_tax_the_model() {
        let (_dir, runtime) = rt();
        let resp = handle_hook(
            &runtime,
            r#"{"hook_event_name":"SessionEnd","session_id":"s1"}"#,
        );
        assert!(resp.stdout.is_empty(), "{}", resp.stdout);
        assert!(!resp.stdout.contains("No context was lost"));
    }

    #[test]
    fn wrapped_shell_post_is_not_ingested_again() {
        let (_dir, runtime) = rt();
        let body = serde_json::json!({
            "hook_event_name": "PostToolUse",
            "session_id": "s1",
            "tool_name": "Bash",
            "tool_input": {"command": "ctx exec --shell -- 'cargo test'"},
            "tool_response": {"stdout": "17 passed, 0 failed\nctx://shell/abc\n", "stderr": ""}
        });
        let resp = handle_hook(&runtime, &body.to_string());
        assert!(resp.stdout.is_empty(), "{}", resp.stdout);
        let obs = runtime.store.observations_for_session("s1").unwrap();
        assert!(obs.is_empty(), "ctx exec already ingested: {obs:?}");
    }

    #[test]
    fn session_start_is_a_short_banner() {
        let (_dir, runtime) = rt();
        let resp = handle_hook(
            &runtime,
            r#"{"hook_event_name":"SessionStart","session_id":"s1","transcript_path":"/tmp/t"}"#,
        );
        assert!(resp.stdout.contains("ctx_fetch"), "{}", resp.stdout);
        assert!(
            !resp.stdout.contains("No context was lost"),
            "{}",
            resp.stdout
        );
    }

    #[test]
    fn session_start_attributes_observations_to_the_reported_model() {
        let (_dir, runtime) = rt();
        handle_hook(
            &runtime,
            r#"{"hook_event_name":"SessionStart","session_id":"model-session","transcript_path":"/tmp/t","model":"claude-sonnet-4-6"}"#,
        );
        runtime
            .ingest(ctx_protocol::CtxEvent::tool_output(
                "model-session",
                ctx_protocol::Harness::ClaudeCode,
                ctx_protocol::ToolRef::new("Bash"),
                fail_log(),
            ))
            .unwrap();

        let rows = runtime.store.dashboard_models(0).unwrap();
        assert_eq!(rows.len(), 1, "{rows:?}");
        assert_eq!(rows[0].id, "claude-sonnet-4-6");
    }

    #[test]
    fn session_start_without_model_is_reported_as_unknown() {
        let (_dir, runtime) = rt();
        handle_hook(
            &runtime,
            r#"{"hook_event_name":"sessionStart","conversation_id":"cursor-session","cursor_version":"1"}"#,
        );
        runtime
            .ingest(ctx_protocol::CtxEvent::tool_output(
                "cursor-session",
                ctx_protocol::Harness::Cursor,
                ctx_protocol::ToolRef::new("Bash"),
                fail_log(),
            ))
            .unwrap();

        let rows = runtime.store.dashboard_models(0).unwrap();
        assert_eq!(rows.len(), 1, "{rows:?}");
        assert_eq!(rows[0].id, "__unknown__");
        assert_eq!(rows[0].source_harnesses, vec!["cursor"]);
    }

    #[test]
    fn a_later_session_start_can_fill_a_missing_model() {
        let (_dir, runtime) = rt();
        handle_hook(
            &runtime,
            r#"{"hook_event_name":"SessionStart","session_id":"resumed","transcript_path":"/tmp/t"}"#,
        );
        handle_hook(
            &runtime,
            r#"{"hook_event_name":"SessionStart","session_id":"resumed","transcript_path":"/tmp/t","model":"claude-opus-4-1"}"#,
        );
        runtime
            .ingest(ctx_protocol::CtxEvent::tool_output(
                "resumed",
                ctx_protocol::Harness::ClaudeCode,
                ctx_protocol::ToolRef::new("Bash"),
                fail_log(),
            ))
            .unwrap();

        let rows = runtime.store.dashboard_models(0).unwrap();
        assert_eq!(rows.len(), 1, "{rows:?}");
        assert_eq!(rows[0].id, "claude-opus-4-1");
    }

    #[test]
    fn prompt_submit_stores_task_without_taxing_the_model() {
        let (_dir, runtime) = rt();
        let body = serde_json::json!({
            "hook_event_name": "UserPromptSubmit",
            "session_id": "s-task",
            "transcript_path": "/tmp/t",
            "prompt": "fix the oauth redirect in auth login"
        });
        let resp = handle_hook(&runtime, &body.to_string());
        assert!(resp.stdout.is_empty(), "{}", resp.stdout);
        let task = runtime.store.session_task("s-task").unwrap();
        assert!(task.contains("oauth"), "{task}");
        assert!(task.contains("auth"), "{task}");
        assert!(!task.contains("fix"), "{task}");
        let pages = runtime.store.page_count().unwrap();
        assert_eq!(pages, 0, "prompts must not become blobs");
    }

    #[test]
    fn session_start_maps_pages_from_another_harness() {
        let (_dir, runtime) = rt();
        let mut payload = String::from("running 40 tests\n");
        for i in 0..40 {
            payload.push_str(&format!("test t{i} ... ok\n"));
        }
        payload.push_str(
            "test auth::login ... FAILED\n\nfailures:\n\n---- auth::login stdout ----\nleft: 401\nright: 200\n",
        );
        let event = ctx_protocol::CtxEvent::tool_output(
            "claude-yesterday",
            ctx_protocol::Harness::ClaudeCode,
            ctx_protocol::ToolRef::new("Bash"),
            payload,
        );
        let ingested = runtime.ingest(event).unwrap();
        let uri = ingested.uri.expect("stored page");
        let resp = handle_hook(
            &runtime,
            r#"{"hook_event_name":"SessionStart","conversation_id":"cursor-today","cursor_version":"1"}"#,
        );
        assert!(resp.stdout.contains("ctx_fetch"), "{}", resp.stdout);
        assert!(
            resp.stdout.contains(&uri) || resp.stdout.contains("claude-code"),
            "cross-harness mapped set missing:\n{}",
            resp.stdout
        );
    }

    fn fail_log() -> String {
        let mut payload = String::from("running 40 tests\n");
        for i in 0..40 {
            payload.push_str(&format!("test t{i} ... ok\n"));
        }
        payload.push_str(
            "test auth::login ... FAILED\n\nfailures:\n\n---- auth::login stdout ----\nleft: 401\nright: 200\n",
        );
        payload
    }

    #[test]
    fn precompact_is_plain_text_keep_list() {
        let (_dir, runtime) = rt();
        runtime
            .ingest(ctx_protocol::CtxEvent::tool_output(
                "s1",
                ctx_protocol::Harness::ClaudeCode,
                ctx_protocol::ToolRef::new("Bash"),
                fail_log(),
            ))
            .unwrap();
        let resp = handle_hook(
            &runtime,
            r#"{"hook_event_name":"PreCompact","session_id":"s1","transcript_path":"/tmp/t","trigger":"auto"}"#,
        );
        assert!(
            !resp.stdout.trim_start().starts_with('{'),
            "{}",
            resp.stdout
        );
        assert!(resp.stdout.contains("ctx://"), "{}", resp.stdout);
        assert!(resp.stdout.contains("ctx_fetch"), "{}", resp.stdout);
        assert!(
            !resp.stdout.contains("hookSpecificOutput"),
            "{}",
            resp.stdout
        );
    }

    #[test]
    fn prompt_after_compact_injects_working_set() {
        let (_dir, runtime) = rt();
        runtime
            .ingest(ctx_protocol::CtxEvent::tool_output(
                "s1",
                ctx_protocol::Harness::ClaudeCode,
                ctx_protocol::ToolRef::new("Bash"),
                fail_log(),
            ))
            .unwrap();
        let _ = handle_hook(
            &runtime,
            r#"{"hook_event_name":"PostCompact","session_id":"s1","transcript_path":"/tmp/t"}"#,
        );
        let body = serde_json::json!({
            "hook_event_name": "UserPromptSubmit",
            "session_id": "s1",
            "transcript_path": "/tmp/t",
            "prompt": "continue"
        });
        let resp = handle_hook(&runtime, &body.to_string());
        assert!(resp.stdout.contains("UserPromptSubmit"), "{}", resp.stdout);
        assert!(resp.stdout.contains("additionalContext"), "{}", resp.stdout);
        assert!(
            resp.stdout.contains("ctx://") || resp.stdout.contains("ctx_fetch"),
            "{}",
            resp.stdout
        );
        let again = handle_hook(&runtime, &body.to_string());
        assert!(
            again.stdout.is_empty(),
            "remap is one-shot:\n{}",
            again.stdout
        );
    }

    #[test]
    fn claude_read_replaces_file_with_outline() {
        let (_dir, runtime) = rt();
        let mut src = String::from("use std::io;\n\n");
        for i in 0..50 {
            src.push_str(&format!("pub fn thing_{i}(x: i32) -> i32 {{ x + {i} }}\n"));
        }
        let body = serde_json::json!({
            "hook_event_name": "PostToolUse",
            "session_id": "s-read",
            "transcript_path": "/tmp/t",
            "tool_name": "Read",
            "tool_input": {"file_path": "src/lib.rs"},
            "tool_response": {"file": {"filePath": "src/lib.rs", "content": src}}
        });
        let resp = handle_hook(&runtime, &body.to_string());
        assert!(resp.stdout.contains("updatedToolOutput"), "{}", resp.stdout);
        assert!(resp.stdout.contains("fn thing_0"), "{}", resp.stdout);
        assert!(!resp.stdout.contains("x + 12"), "{}", resp.stdout);
        assert!(resp.stdout.contains("ctx://"), "{}", resp.stdout);
    }
}
