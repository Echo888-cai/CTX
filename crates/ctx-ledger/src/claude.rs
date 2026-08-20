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
    let root = home.join(".claude").join("projects");
    if !root.is_dir() {
        return report;
    }
    let Ok(projects) = fs::read_dir(&root) else {
        return report;
    };
    for entry in projects.flatten() {
        let path = entry.path();
        if path.is_file() && path.extension().and_then(|s| s.to_str()) == Some("jsonl") {
            report.merge(sync_file(store, &path));
        } else if path.is_dir() {
            if let Ok(files) = fs::read_dir(&path) {
                for f in files.flatten() {
                    let p = f.path();
                    if p.extension().and_then(|s| s.to_str()) == Some("jsonl") {
                        report.merge(sync_file(store, &p));
                    }
                }
            }
        }
    }
    report
}

fn sync_file(store: &Store, path: &Path) -> SyncReport {
    let mut report = SyncReport { files: 1, ..SyncReport::default() };
    let Ok(text) = fs::read_to_string(path) else {
        report.errors.push(format!("read {}", path.display()));
        return report;
    };
    let turns = parse_jsonl(&text, path);
    let mut inner = ingest_turns(store, &turns);
    inner.files = 1;
    inner
}

pub fn parse_jsonl(text: &str, path: &Path) -> Vec<LedgerTurn> {
    let mut turns = Vec::new();
    let session = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_string();
    for line in text.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if let Some(turn) = turn_from_value(&v, &session, path) {
            turns.push(turn);
        }
    }
    turns
}

fn turn_from_value(v: &Value, session: &str, path: &Path) -> Option<LedgerTurn> {
    let message = v.get("message").unwrap_or(v);
    let usage = message.get("usage").or_else(|| v.get("usage"))?;
    if !usage.is_object() {
        return None;
    }
    let model_raw = message
        .get("model")
        .or_else(|| v.get("model"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    if model_raw.is_empty() && usage.get("input_tokens").is_none() {
        return None;
    }
    let parsed = parse_model(&model_raw);
    let ts = v
        .get("timestamp")
        .and_then(Value::as_str)
        .and_then(now_unix_from_rfc3339)
        .or_else(|| v.get("timestamp").and_then(Value::as_i64))
        .unwrap_or(0);
    let cache = usage.get("cache_creation").cloned().unwrap_or(Value::Null);
    Some(LedgerTurn {
        ts,
        harness: "claude-code".into(),
        session: session.to_string(),
        cwd: String::new(),
        model_raw,
        model_base: parsed.base,
        effort: parsed.effort,
        provider: if parsed.provider.is_empty() {
            "anthropic".into()
        } else {
            parsed.provider
        },
        input_tokens: json_i64(usage, "input_tokens"),
        output_tokens: json_i64(usage, "output_tokens"),
        cache_read_tokens: json_i64(usage, "cache_read_input_tokens"),
        cache_write_5m: {
            let w5 = json_i64(&cache, "ephemeral_5m_input_tokens");
            if w5 > 0 {
                w5
            } else {
                json_i64(usage, "cache_creation_input_tokens")
            }
        },
        cache_write_1h: json_i64(&cache, "ephemeral_1h_input_tokens"),
        reasoning_tokens: usage
            .get("output_tokens_details")
            .and_then(|d| d.get("thinking_tokens"))
            .and_then(Value::as_i64)
            .unwrap_or(0),
        context_window: 0,
        is_compaction: v
            .get("type")
            .and_then(Value::as_str)
            .is_some_and(|t| t.contains("compact")),
        quota_used_pct: None,
        plan_type: usage
            .get("service_tier")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        confidence: if parsed.raw.is_empty() {
            "partial".into()
        } else {
            "measured".into()
        },
        source_path: path.display().to_string(),
        resets_at: String::new(),
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
    fn parses_anthropic_usage() {
        let line = r#"{"timestamp":"2026-08-18T10:24:23Z","message":{"model":"claude-sonnet-4-6","usage":{"input_tokens":31291,"cache_creation_input_tokens":100,"cache_read_input_tokens":28000,"output_tokens":244,"output_tokens_details":{"thinking_tokens":12},"cache_creation":{"ephemeral_5m_input_tokens":100,"ephemeral_1h_input_tokens":0}}}}"#;
        let turns = parse_jsonl(line, &PathBuf::from("/tmp/sess.jsonl"));
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].cache_read_tokens, 28000);
        assert_eq!(turns[0].cache_write_5m, 100);
        assert_eq!(turns[0].model_base, "claude-sonnet-4-6");
        assert_eq!(turns[0].confidence, "measured");
        assert_eq!(turns[0].reasoning_tokens, 12);
    }
}
