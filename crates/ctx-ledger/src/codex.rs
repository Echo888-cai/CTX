use std::fs;
use std::path::Path;

use ctx_store::{LedgerTurn, Store};
use serde_json::Value;

use crate::{ingest_turns, now_unix_from_rfc3339, parse_model, SyncReport};

pub fn sync(store: &Store) -> SyncReport {
    let mut report = SyncReport::default();
    let Some(home) = dirs::home_dir() else {
        return report;
    };
    let root = home.join(".codex").join("sessions");
    walk_jsonl(&root, store, &mut report);
    report
}

fn walk_jsonl(dir: &Path, store: &Store, report: &mut SyncReport) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk_jsonl(&path, store, report);
        } else if path.extension().and_then(|s| s.to_str()) == Some("jsonl") {
            report.merge(sync_file(store, &path));
        }
    }
}

fn sync_file(store: &Store, path: &Path) -> SyncReport {
    let Ok(text) = fs::read_to_string(path) else {
        return SyncReport {
            files: 1,
            errors: vec![format!("read {}", path.display())],
            ..SyncReport::default()
        };
    };
    let turns = parse_jsonl(&text, path);
    let mut inner = ingest_turns(store, &turns);
    inner.files = 1;
    inner
}

pub fn parse_jsonl(text: &str, path: &Path) -> Vec<LedgerTurn> {
    let session = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_string();
    let mut model = String::new();
    let mut turns = Vec::new();
    for line in text.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if let Some(m) = v
            .get("payload")
            .and_then(|p| p.get("model"))
            .and_then(Value::as_str)
            .or_else(|| v.get("model").and_then(Value::as_str))
        {
            if !m.is_empty() {
                model = m.to_string();
            }
        }
        let ty = v.get("type").and_then(Value::as_str).unwrap_or("");
        if ty == "compacted" {
            turns.push(LedgerTurn {
                ts: v
                    .get("timestamp")
                    .and_then(Value::as_str)
                    .and_then(now_unix_from_rfc3339)
                    .unwrap_or(0),
                harness: "codex".into(),
                session: session.clone(),
                is_compaction: true,
                confidence: "measured".into(),
                source_path: path.display().to_string(),
                ..LedgerTurn::default()
            });
            continue;
        }
        if let Some(turn) = token_count(&v, &session, &model, path) {
            turns.push(turn);
        }
    }
    turns
}

fn token_count(v: &Value, session: &str, model: &str, path: &Path) -> Option<LedgerTurn> {
    let payload = v.get("payload")?;
    if payload.get("type").and_then(Value::as_str) != Some("token_count") {
        return None;
    }
    let info = payload.get("info")?;
    let last = info
        .get("last_token_usage")
        .or_else(|| info.get("total_token_usage"))?;
    let parsed = parse_model(model);
    let rates = v.get("payload").and_then(|p| p.get("rate_limits"));
    Some(LedgerTurn {
        ts: v
            .get("timestamp")
            .and_then(Value::as_str)
            .and_then(now_unix_from_rfc3339)
            .unwrap_or(0),
        harness: "codex".into(),
        session: session.to_string(),
        cwd: String::new(),
        model_raw: model.to_string(),
        model_base: parsed.base,
        effort: parsed.effort,
        provider: if parsed.provider.is_empty() {
            "openai".into()
        } else {
            parsed.provider
        },
        input_tokens: json_i64(last, "input_tokens"),
        output_tokens: json_i64(last, "output_tokens"),
        cache_read_tokens: json_i64(last, "cached_input_tokens"),
        cache_write_5m: json_i64(last, "cache_write_input_tokens"),
        cache_write_1h: 0,
        reasoning_tokens: json_i64(last, "reasoning_output_tokens"),
        context_window: info
            .get("model_context_window")
            .and_then(Value::as_i64)
            .unwrap_or(0),
        is_compaction: false,
        quota_used_pct: rates
            .and_then(|r| r.get("primary"))
            .and_then(|p| p.get("used_percent"))
            .and_then(Value::as_f64),
        plan_type: rates
            .and_then(|r| r.get("plan_type"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        resets_at: rates
            .and_then(|r| r.get("primary"))
            .and_then(|p| p.get("resets_at").or_else(|| p.get("resetsAt")))
            .map(|v| match v {
                Value::String(s) => s.clone(),
                Value::Number(n) => n.to_string(),
                _ => String::new(),
            })
            .unwrap_or_default(),
        confidence: if model.is_empty() {
            "partial".into()
        } else {
            "measured".into()
        },
        source_path: path.display().to_string(),
    })
}

fn json_i64(v: &Value, key: &str) -> i64 {
    v.get(key).and_then(Value::as_i64).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn parses_codex_token_count() {
        let line = r#"{"timestamp":"2026-08-18T10:24:23.458Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":26031,"cached_input_tokens":11008,"cache_write_input_tokens":0,"output_tokens":431,"reasoning_output_tokens":131},"model_context_window":258400},"rate_limits":{"primary":{"used_percent":66.0},"plan_type":"plus"}}}"#;
        let meta = r#"{"type":"session_meta","payload":{"model":"gpt-5.6-sol"}}"#;
        let turns = parse_jsonl(&format!("{meta}\n{line}"), &PathBuf::from("rollout.jsonl"));
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].cache_read_tokens, 11008);
        assert_eq!(turns[0].quota_used_pct, Some(66.0));
        assert_eq!(turns[0].plan_type, "plus");
        assert_eq!(turns[0].model_base, "gpt-5.6-sol");
    }
}
