use std::path::Path;

use serde_json::Value;

use ctx_core::CtxPaths;

#[derive(Debug, Clone)]
pub struct Check {
    pub ok: bool,
    pub name: &'static str,
    pub detail: String,
}

#[derive(Debug, Clone)]
pub struct DoctorReport {
    pub checks: Vec<Check>,
}

impl DoctorReport {
    pub fn render(&self) -> String {
        let mut lines = vec!["CTX doctor".to_string(), String::new()];
        for c in &self.checks {
            let mark = if c.ok { "✓" } else { "·" };
            lines.push(format!("  {mark}  {:<10} {}", c.name, c.detail));
        }
        lines.push(String::new());
        if self.checks.iter().any(|c| !c.ok && c.name == "binary") {
            lines.push("Next: cargo install --path apps/cli".into());
        } else if self
            .checks
            .iter()
            .any(|c| !c.ok && matches!(c.name, "claude" | "cursor" | "windsurf" | "mcp"))
        {
            lines.push("Next: ctx setup claude, cursor, or windsurf".into());
        } else if self.checks.iter().any(|c| !c.ok && c.name == "database") {
            lines.push("Next: ctx init".into());
        } else {
            lines.push("ctx is wired. Try ctx demo · ctx inspect.".into());
        }
        lines.join("\n")
    }
}

pub fn run() -> anyhow::Result<()> {
    let report = collect()?;
    println!("{}", report.render());
    Ok(())
}

pub fn collect() -> anyhow::Result<DoctorReport> {
    let paths = CtxPaths::default_home()?;
    let home = dirs::home_dir();
    Ok(DoctorReport {
        checks: vec![
            binary_check(),
            store_check(&paths),
            db_check(&paths),
            claude_hooks_check(home.as_deref()),
            cursor_hooks_check(home.as_deref()),
            windsurf_mcp_check(home.as_deref()),
            mcp_check(home.as_deref()),
        ],
    })
}

fn binary_check() -> Check {
    match std::env::current_exe() {
        Ok(p) => Check {
            ok: p.exists(),
            name: "binary",
            detail: p.display().to_string(),
        },
        Err(_) => Check {
            ok: false,
            name: "binary",
            detail: "unknown path".into(),
        },
    }
}

fn store_check(paths: &CtxPaths) -> Check {
    let dir = paths.store_dir();
    let detail = if paths.fallback {
        format!("{}  (workspace — home not writable)", dir.display())
    } else {
        dir.display().to_string()
    };
    Check {
        ok: dir.is_dir(),
        name: "store",
        detail,
    }
}

fn db_check(paths: &CtxPaths) -> Check {
    let db = paths.db_path();
    Check {
        ok: db.is_file(),
        name: "database",
        detail: db.display().to_string(),
    }
}

fn claude_hooks_check(home: Option<&Path>) -> Check {
    let Some(home) = home else {
        return Check {
            ok: false,
            name: "claude",
            detail: "no home directory".into(),
        };
    };
    let settings = home.join(".claude").join("settings.json");
    match read_object(&settings) {
        Ok(Some(v)) => {
            let ok = hooks_contain_ctx(v.get("hooks"));
            if ok {
                if let Some(missing) = first_stale_hook_bin(v.get("hooks")) {
                    return Check {
                        ok: false,
                        name: "claude",
                        detail: format!("stale binary {missing} — ctx setup claude"),
                    };
                }
            }
            Check {
                ok,
                name: "claude",
                detail: if ok {
                    settings.display().to_string()
                } else {
                    format!("{}  (hooks missing)", settings.display())
                },
            }
        }
        Ok(None) => Check {
            ok: false,
            name: "claude",
            detail: "not installed".into(),
        },
        Err(_) => Check {
            ok: false,
            name: "claude",
            detail: format!("{}  (unreadable)", settings.display()),
        },
    }
}

fn cursor_hooks_check(home: Option<&Path>) -> Check {
    let Some(home) = home else {
        return Check {
            ok: false,
            name: "cursor",
            detail: "no home directory".into(),
        };
    };
    let hooks = home.join(".cursor").join("hooks.json");
    match read_object(&hooks) {
        Ok(Some(v)) => {
            let ok = hooks_contain_ctx(v.get("hooks"));
            if ok {
                if let Some(missing) = first_stale_hook_bin(v.get("hooks")) {
                    return Check {
                        ok: false,
                        name: "cursor",
                        detail: format!("stale binary {missing} — ctx setup cursor"),
                    };
                }
            }
            Check {
                ok,
                name: "cursor",
                detail: if ok {
                    hooks.display().to_string()
                } else {
                    format!("{}  (hooks missing)", hooks.display())
                },
            }
        }
        Ok(None) => Check {
            ok: false,
            name: "cursor",
            detail: "not installed".into(),
        },
        Err(_) => Check {
            ok: false,
            name: "cursor",
            detail: format!("{}  (unreadable)", hooks.display()),
        },
    }
}

fn windsurf_mcp_check(home: Option<&Path>) -> Check {
    let Some(home) = home else {
        return Check {
            ok: false,
            name: "windsurf",
            detail: "no home directory".into(),
        };
    };
    let path = home
        .join(".codeium")
        .join("windsurf")
        .join("mcp_config.json");
    let present = path.exists()
        || Path::new("/Applications/Windsurf.app").exists()
        || home.join(".codeium").join("windsurf").exists();
    if !present {
        return Check {
            ok: true,
            name: "windsurf",
            detail: "not installed".into(),
        };
    }
    if !path.exists() {
        return Check {
            ok: false,
            name: "windsurf",
            detail: "no mcp_config.json — ctx setup windsurf".into(),
        };
    }
    match read_object(&path) {
        Ok(Some(v)) if mcp_registered(&v) => Check {
            ok: true,
            name: "windsurf",
            detail: "~/.codeium/windsurf/mcp_config.json".into(),
        },
        _ => Check {
            ok: false,
            name: "windsurf",
            detail: "mcp not registered — ctx setup windsurf".into(),
        },
    }
}

fn mcp_check(home: Option<&Path>) -> Check {
    let Some(home) = home else {
        return Check {
            ok: false,
            name: "mcp",
            detail: "no home directory".into(),
        };
    };
    let candidates = [
        home.join(".claude").join("settings.json"),
        home.join(".claude.json"),
        home.join(".cursor").join("mcp.json"),
        home.join(".codeium")
            .join("windsurf")
            .join("mcp_config.json"),
    ];
    let mut found = Vec::new();
    for path in &candidates {
        if let Ok(Some(v)) = read_object(path) {
            if mcp_registered(&v) {
                found.push(label_for(path));
            }
        }
    }
    if found.is_empty() {
        Check {
            ok: false,
            name: "mcp",
            detail: "not registered".into(),
        }
    } else {
        Check {
            ok: true,
            name: "mcp",
            detail: found.join(", "),
        }
    }
}

fn label_for(path: &Path) -> String {
    let s = path.to_string_lossy();
    if s.contains(".cursor") {
        "cursor".into()
    } else if s.contains("windsurf") {
        "windsurf".into()
    } else if s.ends_with(".claude.json") {
        "claude.json".into()
    } else {
        "claude".into()
    }
}

fn read_object(path: &Path) -> Result<Option<Value>, ()> {
    if !path.exists() {
        return Ok(None);
    }
    let bytes = std::fs::read(path).map_err(|_| ())?;
    let v: Value = serde_json::from_slice(&bytes).map_err(|_| ())?;
    if v.is_object() {
        Ok(Some(v))
    } else {
        Err(())
    }
}

/// Walk hook trees only — never stringify whole settings (may contain secrets).
pub fn hooks_contain_ctx(hooks: Option<&Value>) -> bool {
    let Some(hooks) = hooks else {
        return false;
    };
    walk_for_ctx_hook(hooks)
}

fn walk_for_ctx_hook(v: &Value) -> bool {
    match v {
        Value::String(s) => is_ctx_hook_command(s),
        Value::Array(arr) => arr.iter().any(walk_for_ctx_hook),
        Value::Object(map) => map.values().any(walk_for_ctx_hook),
        _ => false,
    }
}

pub fn is_ctx_hook_command(s: &str) -> bool {
    s.contains("ctx hook") || s.contains("ctx' hook") || s.contains("ctx hook skipped")
}

fn first_stale_hook_bin(hooks: Option<&Value>) -> Option<String> {
    let cmd = first_ctx_hook_command(hooks?)?;
    stale_hook_binary(&cmd)
}

fn first_ctx_hook_command(v: &Value) -> Option<String> {
    match v {
        Value::String(s) if is_ctx_hook_command(s) => Some(s.clone()),
        Value::Array(arr) => arr.iter().find_map(first_ctx_hook_command),
        Value::Object(map) => map.values().find_map(first_ctx_hook_command),
        _ => None,
    }
}

/// `if [ -x /path/ctx ]; then /path/ctx hook` — missing PATH means setup is stale.
pub fn stale_hook_binary(cmd: &str) -> Option<String> {
    let rest = cmd.split("if [ -x ").nth(1)?;
    let raw = rest.split(']').next()?.trim();
    let path = raw.trim_matches('\'').trim_matches('"');
    if path.is_empty() {
        return None;
    }
    if Path::new(path).is_file() {
        None
    } else {
        Some(path.to_string())
    }
}

pub fn mcp_registered(root: &Value) -> bool {
    root.get("mcpServers").and_then(|s| s.get("ctx")).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn parse_report_lines(rendered: &str) -> Vec<(char, String)> {
        rendered
            .lines()
            .filter_map(|line| {
                let t = line.trim_start();
                let mark = t.chars().next()?;
                if mark != '✓' && mark != '·' {
                    return None;
                }
                let rest = t.get(mark.len_utf8()..)?.trim();
                let name = rest.split_whitespace().next()?.to_string();
                Some((mark, name))
            })
            .collect()
    }

    #[test]
    fn detects_claude_hooks_without_dumping_secrets() {
        let settings = json!({
            "env": { "ANTHROPIC_API_KEY": "sk-secret" },
            "hooks": {
                "PostToolUse": [{ "hooks": [{ "type": "command", "command": "/opt/ctx hook" }] }]
            }
        });
        assert!(hooks_contain_ctx(settings.get("hooks")));
        assert!(!hooks_contain_ctx(settings.get("env")));
        let rendered = DoctorReport {
            checks: vec![Check {
                ok: true,
                name: "claude",
                detail: "~/.claude/settings.json".into(),
            }],
        }
        .render();
        assert!(!rendered.contains("sk-secret"));
        assert!(rendered.contains("✓"));
    }

    #[test]
    fn wired_report_points_at_inspect() {
        let rendered = DoctorReport {
            checks: vec![Check {
                ok: true,
                name: "binary",
                detail: "/opt/ctx".into(),
            }],
        }
        .render();
        assert!(rendered.contains("ctx is wired") || rendered.contains("ctx inspect"));
        assert!(!rendered.contains("Next: ctx init"), "{rendered}");
    }

    #[test]
    fn parse_report_lines_reads_marks() {
        let report = DoctorReport {
            checks: vec![
                Check {
                    ok: true,
                    name: "binary",
                    detail: "/opt/ctx".into(),
                },
                Check {
                    ok: false,
                    name: "mcp",
                    detail: "not registered".into(),
                },
            ],
        };
        let lines = parse_report_lines(&report.render());
        assert_eq!(lines, vec![('✓', "binary".into()), ('·', "mcp".into())]);
    }

    #[test]
    fn mcp_registered_looks_at_ctx_server_only() {
        assert!(mcp_registered(
            &json!({"mcpServers": {"ctx": {"command": "/opt/ctx"}}})
        ));
        assert!(!mcp_registered(&json!({"mcpServers": {"other": {}}})));
        assert!(!mcp_registered(&json!({})));
    }

    #[test]
    fn quoted_absolute_hook_still_matches() {
        assert!(is_ctx_hook_command("'/opt/my ctx' hook"));
        assert!(is_ctx_hook_command("/opt/ctx hook"));
        assert!(!is_ctx_hook_command("echo hello"));
    }

    #[test]
    fn stale_hook_binary_detects_missing_path() {
        let missing = stale_hook_binary(
            "if [ -x /no/such/ctx-bin-xyz ]; then /no/such/ctx-bin-xyz hook; else echo skip; fi",
        );
        assert_eq!(missing.as_deref(), Some("/no/such/ctx-bin-xyz"));
        let cmd = hook_cmd_for_existing();
        assert!(stale_hook_binary(&cmd).is_none(), "{cmd}");
    }

    fn hook_cmd_for_existing() -> String {
        let exe = std::env::current_exe().unwrap();
        format!(
            "if [ -x {p} ]; then {p} hook; else echo skip; fi",
            p = exe.display()
        )
    }
}
