use serde_json::{json, Value};

use ctx_core::{Config, CtxPaths, Runtime, Snapshot};

use crate::doctor::{
    cursor_registered_levels, first_stale_hook_bin, hooks_contain_ctx, mcp_registered,
};
use crate::setup::{
    claude_desktop_config_path, detect_aider, detect_claude, detect_claude_desktop, detect_codex,
    detect_continue, detect_cursor, detect_jetbrains, detect_vscode, detect_windsurf,
    read_json_object,
};

#[derive(Clone, Copy)]
struct Spec {
    id: &'static str,
    name: &'static str,
    form: &'static str,
    integration: &'static str,
    capability: &'static str,
    config_paths: &'static [&'static str],
    shared_with: &'static [&'static str],
}

const SPECS: &[Spec] = &[
    Spec {
        id: "cursor",
        name: "Cursor",
        form: "desktop+cli",
        integration: "hooks",
        capability: "auto",
        config_paths: &["~/.cursor/hooks.json", "~/.cursor/mcp.json"],
        shared_with: &["cursor-cli"],
    },
    Spec {
        id: "claude-code",
        name: "Claude Code",
        form: "cli",
        integration: "hooks",
        capability: "auto",
        config_paths: &["~/.claude/settings.json", "~/.claude.json"],
        shared_with: &[],
    },
    Spec {
        id: "claude-desktop",
        name: "Claude Desktop",
        form: "desktop",
        integration: "mcp",
        capability: "retrieval",
        config_paths: &["~/Library/Application Support/Claude/claude_desktop_config.json"],
        shared_with: &[],
    },
    Spec {
        id: "codex",
        name: "Codex",
        form: "desktop+cli",
        integration: "mcp",
        capability: "retrieval",
        config_paths: &["~/.codex/config.toml"],
        shared_with: &[],
    },
    Spec {
        id: "windsurf",
        name: "Windsurf",
        form: "desktop",
        integration: "mcp",
        capability: "retrieval",
        config_paths: &["~/.codeium/windsurf/mcp_config.json"],
        shared_with: &[],
    },
    Spec {
        id: "vscode",
        name: "VS Code",
        form: "desktop",
        integration: "mcp",
        capability: "retrieval",
        config_paths: &["~/Library/Application Support/Code/User/mcp.json"],
        shared_with: &[],
    },
    Spec {
        id: "continue",
        name: "Continue.dev",
        form: "plugin",
        integration: "mcp",
        capability: "retrieval",
        config_paths: &["~/.continue/config.yaml"],
        shared_with: &[],
    },
    Spec {
        id: "jetbrains",
        name: "JetBrains",
        form: "desktop",
        integration: "mcp",
        capability: "retrieval",
        config_paths: &["~/Library/Application Support/JetBrains/mcp.json"],
        shared_with: &[],
    },
    Spec {
        id: "aider",
        name: "Aider",
        form: "cli",
        integration: "wrapper",
        capability: "retrieval",
        config_paths: &[],
        shared_with: &[],
    },
    Spec {
        id: "copilot",
        name: "GitHub Copilot",
        form: "plugin",
        integration: "mcp",
        capability: "retrieval",
        config_paths: &[],
        shared_with: &["vscode"],
    },
];

const SURFACE: &[&str] = &["cursor", "claude-code", "codex"];

pub fn payload() -> Value {
    let home = dirs::home_dir();
    let cfg = CtxPaths::default_home()
        .ok()
        .map(|paths| Config::load(&paths));
    let today = Runtime::open_default()
        .ok()
        .and_then(|rt| Snapshot::capture(&rt.store).ok())
        .map(|s| s.by_harness_today)
        .unwrap_or_default();
    let harnesses: Vec<Value> = SPECS
        .iter()
        .filter(|spec| SURFACE.contains(&spec.id))
        .map(|spec| {
            let detected = detect(spec.id);
            let (installed, stale, levels) = install_state(spec, home.as_deref());
            let enabled = cfg
                .as_ref()
                .map(|c| !c.disabled_harnesses.iter().any(|id| id == spec.id || (spec.id == "claude-code" && id == "claude")))
                .unwrap_or(true);
            let stats = today.iter().find(|(name, _)| harness_ids_match(spec.id, name));
            json!({
                "id": spec.id,
                "name": spec.name,
                "form": spec.form,
                "form_label": form_label(spec.form),
                "integration": spec.integration,
                "capability": spec.capability,
                "detected": detected,
                "installed": installed,
                "stale": stale,
                "enabled": enabled,
                "shared_with": spec.shared_with,
                "config_paths": spec.config_paths,
                "registered_levels": levels,
                "status": status_line(installed, spec.capability),
                "today": stats.map(|(_, t)| json!({
                    "raw": t.raw,
                    "delivered": t.delivered,
                    "avoided": t.avoided,
                    "refetched": t.refetched,
                    "net_avoided": t.net_avoided(),
                    "reduction_pct": t.reduction_pct(),
                })).unwrap_or(json!({
                    "raw": 0, "delivered": 0, "avoided": 0,
                    "refetched": 0, "net_avoided": 0, "reduction_pct": 0
                })),
            })
        })
        .collect();
    json!({ "ok": true, "harnesses": harnesses })
}

fn form_label(form: &str) -> &'static str {
    match form {
        "desktop+cli" => "Desktop / CLI",
        "desktop" => "Desktop",
        "cli" => "CLI",
        _ => "CLI",
    }
}

fn status_line(installed: bool, capability: &str) -> &'static str {
    if !installed {
        "未安装"
    } else if capability == "auto" {
        "hooks 已接入"
    } else {
        "仅 MCP，模型主动检索才省"
    }
}

pub fn summary_json() -> Value {
    let rows = payload()["harnesses"].as_array().cloned().unwrap_or_default();
    let auto: Vec<&Value> = rows
        .iter()
        .filter(|h| h["capability"] == "auto" && h["installed"] == true && h["enabled"] != false)
        .collect();
    let retrieval_only = rows
        .iter()
        .filter(|h| h["capability"] == "retrieval" && h["installed"] == true)
        .count();
    let names: Vec<String> = auto
        .iter()
        .filter_map(|h| h["name"].as_str().map(str::to_string))
        .collect();
    json!({
        "auto_on": auto.len(),
        "auto_total": rows.iter().filter(|h| h["capability"] == "auto").count(),
        "installed": rows.iter().filter(|h| h["installed"] == true).count(),
        "on": rows.iter().filter(|h| h["installed"] == true && h["enabled"] != false).count(),
        "retrieval": retrieval_only,
        "names": names,
        "total": rows.len(),
        "tools": rows,
    })
}

pub fn set_enabled(id: &str, enabled: bool) -> Value {
    if !SPECS.iter().any(|s| s.id == id) {
        return json!({"ok": false, "error": format!("unknown harness {id}")});
    }
    let Ok(paths) = CtxPaths::default_home() else {
        return json!({"ok": false, "error": "no home"});
    };
    let mut cfg = Config::load(&paths);
    if enabled {
        cfg.disabled_harnesses.retain(|item| item != id);
    } else if !cfg.disabled_harnesses.iter().any(|item| item == id) {
        cfg.disabled_harnesses.push(id.to_string());
    }
    match cfg.save(&paths) {
        Ok(()) => json!({"ok": true, "id": id, "enabled": enabled}),
        Err(err) => json!({"ok": false, "error": err.to_string()}),
    }
}

pub fn capability(id: &str) -> &'static str {
    SPECS
        .iter()
        .find(|s| s.id == id || (id == "claude" && s.id == "claude-code"))
        .map(|s| s.capability)
        .unwrap_or("retrieval")
}

pub fn display_name(id: &str) -> String {
    SPECS
        .iter()
        .find(|s| s.id == id || (id == "claude" && s.id == "claude-code"))
        .map(|s| s.name.to_string())
        .unwrap_or_else(|| id.to_string())
}

fn detect(id: &str) -> bool {
    match id {
        "cursor" => detect_cursor(),
        "claude-code" => detect_claude(),
        "claude-desktop" => detect_claude_desktop(),
        "codex" => detect_codex(),
        "windsurf" => detect_windsurf(),
        "vscode" | "copilot" => detect_vscode(),
        "continue" => detect_continue(),
        "jetbrains" => detect_jetbrains(),
        "aider" => detect_aider(),
        _ => false,
    }
}

fn install_state(spec: &Spec, home: Option<&std::path::Path>) -> (bool, bool, Vec<String>) {
    let Some(home) = home else {
        return (false, false, vec![]);
    };
    match spec.id {
        "cursor" => {
            let hooks = home.join(".cursor").join("hooks.json");
            let mcp = home.join(".cursor").join("mcp.json");
            let installed = file_has_hooks(&hooks) || file_has_mcp(&mcp);
            let stale = file_stale(&hooks);
            (installed, stale, cursor_registered_levels(home))
        }
        "claude-code" => {
            let settings = home.join(".claude").join("settings.json");
            let installed = file_has_hooks(&settings) || file_has_mcp(&settings);
            (installed, file_stale(&settings), vec!["user".into()])
        }
        "claude-desktop" => {
            let path = claude_desktop_config_path(home);
            (file_has_mcp(&path), false, vec![])
        }
        "codex" => {
            let path = home.join(".codex").join("config.toml");
            let text = std::fs::read_to_string(path).unwrap_or_default();
            (text.contains("[mcp_servers.ctx]"), false, vec![])
        }
        "windsurf" => {
            let path = home
                .join(".codeium")
                .join("windsurf")
                .join("mcp_config.json");
            (file_has_mcp(&path), false, vec![])
        }
        "vscode" | "copilot" => {
            let path = home
                .join("Library")
                .join("Application Support")
                .join("Code")
                .join("User")
                .join("mcp.json");
            (file_has_mcp(&path), false, vec![])
        }
        "continue" => {
            let path = home.join(".continue").join("mcpServers").join("ctx.yaml");
            (path.is_file(), false, vec![])
        }
        "jetbrains" => {
            let path = home
                .join("Library")
                .join("Application Support")
                .join("JetBrains")
                .join("mcp.json");
            (file_has_mcp(&path), false, vec![])
        }
        "aider" => {
            let wrap = CtxPaths::default_home()
                .ok()
                .map(|p| p.root().join("bin").join("aider-ctx"))
                .filter(|p| p.is_file());
            (wrap.is_some(), false, vec![])
        }
        _ => (false, false, vec![]),
    }
}

fn file_has_hooks(path: &std::path::Path) -> bool {
    read_json_object(path)
        .ok()
        .map(|v| hooks_contain_ctx(v.get("hooks")))
        .unwrap_or(false)
}

fn file_has_mcp(path: &std::path::Path) -> bool {
    read_json_object(path)
        .ok()
        .map(|v| mcp_registered(&v))
        .unwrap_or(false)
}

fn file_stale(path: &std::path::Path) -> bool {
    read_json_object(path)
        .ok()
        .and_then(|v| first_stale_hook_bin(v.get("hooks")))
        .is_some()
}

fn harness_ids_match(spec_id: &str, stored: &str) -> bool {
    spec_id == stored
        || (spec_id == "claude-code" && matches!(stored, "claude" | "claude-code"))
        || (spec_id == "cursor" && stored == "cursor-cli")
}
