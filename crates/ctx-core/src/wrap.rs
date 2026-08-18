//! Rewrite a harness shell command so CTX executes it and returns a compact result.

const ALWAYS_INTERACTIVE: &[&str] = &[
    "vim", "nvim", "vi", "nano", "emacs", "less", "more", "top", "htop", "ssh", "mysql", "psql",
    "sqlite3", "ipython", "irb", "prisma",
];

const REPL: &[&str] = &["python", "python3", "node", "nodejs"];

#[derive(Debug, Clone)]
pub struct WrappedCommand {
    pub command: String,
    pub wrapped: bool,
}

pub fn wrap_shell_command(original: &str) -> WrappedCommand {
    wrap_shell_command_with(original, resolve_ctx_bin().as_deref())
}

pub fn wrap_shell_command_with(original: &str, ctx_bin: Option<&str>) -> WrappedCommand {
    let trimmed = original.trim();
    if trimmed.is_empty() || is_already_wrapped(trimmed) || is_interactive(trimmed) {
        return WrappedCommand {
            command: original.to_string(),
            wrapped: false,
        };
    }
    let Some(bin) = ctx_bin.filter(|s| !s.is_empty()) else {
        tracing::warn!(
            "ctx: shell wrap skipped — ctx binary not found. Tool runs unmodified (fail-open)."
        );
        return WrappedCommand {
            command: original.to_string(),
            wrapped: false,
        };
    };
    WrappedCommand {
        command: format!(
            "{} exec --shell -- {}",
            format_bin(bin),
            single_quote(trimmed)
        ),
        wrapped: true,
    }
}

pub fn rewrite_shell_command(tool_input: &serde_json::Value) -> Option<serde_json::Value> {
    rewrite_shell_command_with(tool_input, resolve_ctx_bin().as_deref())
}

pub fn rewrite_shell_command_with(
    tool_input: &serde_json::Value,
    ctx_bin: Option<&str>,
) -> Option<serde_json::Value> {
    let cmd = tool_input.get("command")?.as_str()?;
    let wrapped = wrap_shell_command_with(cmd, ctx_bin);
    if !wrapped.wrapped {
        return None;
    }
    let mut updated = tool_input.clone();
    if let Some(obj) = updated.as_object_mut() {
        obj.insert("command".into(), serde_json::Value::String(wrapped.command));
        return Some(updated);
    }
    None
}

/// Prefer the running `ctx` binary so hooks don't depend on PATH.
pub fn resolve_ctx_bin() -> Option<String> {
    if let Ok(p) = std::env::var("CTX_BIN") {
        if !p.is_empty() {
            return Some(p);
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(name) = exe.file_name().and_then(|n| n.to_str()) {
            if name == "ctx" || name == "ctx.exe" {
                return Some(exe.display().to_string());
            }
        }
    }
    None
}

pub fn is_already_wrapped(cmd: &str) -> bool {
    let t = cmd.trim_start();
    t.starts_with("ctx exec") || t.contains("ctx exec --shell --")
}

fn format_bin(bin: &str) -> String {
    if bin.chars().any(|c| c.is_whitespace() || c == '\'') {
        single_quote(bin)
    } else {
        bin.to_string()
    }
}

fn is_interactive(cmd: &str) -> bool {
    let parts: Vec<&str> = cmd.split_whitespace().collect();
    let first = parts.first().copied().unwrap_or("");
    let base = first.rsplit('/').next().unwrap_or(first);
    if ALWAYS_INTERACTIVE.contains(&base) {
        return true;
    }
    if REPL.contains(&base) {
        // Bare REPL, or `python -i`. `python -m pytest` / `node app.js` wrap.
        return parts.len() <= 1 || matches!(parts.get(1).copied(), Some("-i" | "--interactive"));
    }
    false
}

pub fn single_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wraps_npm_test() {
        let w = wrap_shell_command_with("npm test", Some("ctx"));
        assert!(w.wrapped);
        assert_eq!(w.command, "ctx exec --shell -- 'npm test'");
    }

    #[test]
    fn quotes_pipelines() {
        let w = wrap_shell_command_with("npm test && echo done", Some("ctx"));
        assert_eq!(w.command, "ctx exec --shell -- 'npm test && echo done'");
    }

    #[test]
    fn wraps_with_absolute_bin() {
        let w = wrap_shell_command_with("ls", Some("/opt/ctx"));
        assert_eq!(w.command, "/opt/ctx exec --shell -- 'ls'");
    }

    #[test]
    fn skips_nested() {
        assert!(!wrap_shell_command_with("ctx exec --shell -- 'ls'", Some("ctx")).wrapped);
        assert!(
            !wrap_shell_command_with("/opt/ctx exec --shell -- 'ls'", Some("/opt/ctx")).wrapped
        );
    }

    #[test]
    fn skips_vim() {
        assert!(!wrap_shell_command_with("vim src/main.rs", Some("ctx")).wrapped);
    }

    #[test]
    fn wraps_python_module_but_skips_repl() {
        let pytest = wrap_shell_command_with("python -m pytest", Some("ctx"));
        assert!(pytest.wrapped, "{}", pytest.command);
        assert!(!wrap_shell_command_with("python", Some("ctx")).wrapped);
        assert!(!wrap_shell_command_with("python3 -i", Some("ctx")).wrapped);
        let node = wrap_shell_command_with("node test.js", Some("ctx"));
        assert!(node.wrapped, "{}", node.command);
        assert!(!wrap_shell_command_with("node", Some("ctx")).wrapped);
    }

    #[test]
    fn fail_open_without_bin() {
        let w = wrap_shell_command_with("npm test", None);
        assert!(!w.wrapped);
        assert_eq!(w.command, "npm test");
    }
}
