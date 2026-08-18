use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::Context;
use ctx_core::{ensure_exec_header, single_quote, Runtime};
use ctx_protocol::{CtxEvent, Harness, ToolRef};

pub fn run(shell: bool, cwd: Option<&Path>, command: &[String]) -> anyhow::Result<i32> {
    if command.is_empty() {
        anyhow::bail!("missing command");
    }

    // Merge stderr into stdout so cargo/npm status lines stay next to test output.
    // Otherwise `Running unittests` (stderr) arrives after all `test ... ok` lines.
    let inner = if shell {
        command.join(" ")
    } else {
        command
            .iter()
            .map(|s| single_quote(s))
            .collect::<Vec<_>>()
            .join(" ")
    };
    let script = format!("{{ {inner}\n}} 2>&1");
    let mut cmd = Command::new("sh");
    cmd.arg("-c").arg(&script);
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    cmd.stdin(Stdio::inherit());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::null());

    let output = cmd
        .output()
        .with_context(|| format!("exec {:?}", command))?;
    let payload = String::from_utf8_lossy(&output.stdout).into_owned();

    let code = output.status.code().unwrap_or(1);
    let display_cmd = command.join(" ");

    match Runtime::open_default() {
        Ok(rt) => {
            let event = CtxEvent {
                event: ctx_protocol::EventKind::ToolOutput,
                session: session_id(),
                harness: detect_harness(),
                tool: Some(ToolRef::new("Bash")),
                payload: payload.clone(),
                task_context: Some(display_cmd.clone()),
                metadata: serde_json::json!({
                    "cwd": cwd.map(|p| p.display().to_string()),
                    "exit_code": code,
                    "command": display_cmd,
                }),
            };
            match rt.ingest(event) {
                Ok(result) => {
                    let out = ensure_exec_header(
                        &display_cmd,
                        code,
                        &result.delivered,
                        result.uri.as_deref(),
                    );
                    print!("{out}");
                    if !out.ends_with('\n') {
                        println!();
                    }
                }
                Err(err) => {
                    tracing::warn!(error = %err, "ctx exec ingest failed; passing raw output");
                    let out = ensure_exec_header(&display_cmd, code, &payload, None);
                    print!("{out}");
                }
            }
        }
        Err(err) => {
            tracing::warn!(error = %err, "ctx store unavailable; passing raw output");
            let out = ensure_exec_header(&display_cmd, code, &payload, None);
            print!("{out}");
        }
    }

    Ok(code)
}

fn session_id() -> String {
    std::env::var("CTX_SESSION")
        .or_else(|_| std::env::var("CLAUDE_SESSION_ID"))
        .unwrap_or_else(|_| format!("exec-{}", std::process::id()))
}

fn detect_harness() -> Harness {
    if std::env::var("CURSOR_TRACE_ID").is_ok() || std::env::var("CURSOR_SESSION_ID").is_ok() {
        Harness::Cursor
    } else if std::env::var("CLAUDE_SESSION_ID").is_ok() || std::env::var("CLAUDECODE").is_ok() {
        Harness::ClaudeCode
    } else {
        Harness::Unknown
    }
}
