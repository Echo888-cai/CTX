use std::path::PathBuf;

use ctx_store::{LedgerTurn, Store};
use rusqlite::{Connection, OpenFlags};
use serde_json::Value;

use crate::{ingest_turns, parse_model, SyncReport};

pub fn sync(store: &Store) -> SyncReport {
    let Some(home) = dirs::home_dir() else {
        return SyncReport::default();
    };
    let db = home
        .join("Library")
        .join("Application Support")
        .join("Cursor")
        .join("User")
        .join("globalStorage")
        .join("state.vscdb");
    if !db.is_file() {
        return SyncReport::default();
    }
    match parse_vscdb(&db) {
        Ok(turns) => {
            let mut report = ingest_turns(store, &turns);
            report.files = 1;
            report
        }
        Err(err) => SyncReport {
            files: 1,
            errors: vec![err],
            ..SyncReport::default()
        },
    }
}

pub fn parse_vscdb(path: &PathBuf) -> Result<Vec<LedgerTurn>, String> {
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare("SELECT key, value FROM cursorDiskKV WHERE key LIKE 'bubbleId:%' LIMIT 8000")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?))
        })
        .map_err(|e| e.to_string())?;
    let mut turns = Vec::new();
    for row in rows.flatten() {
        let (key, Some(value)) = row else { continue };
        let Ok(v) = serde_json::from_str::<Value>(&value) else {
            continue;
        };
        if let Some(turn) = from_bubble(&key, &v, path) {
            turns.push(turn);
        }
    }
    Ok(turns)
}

fn from_bubble(key: &str, v: &Value, path: &PathBuf) -> Option<LedgerTurn> {
    let tokens = v.get("tokenCount")?;
    let input = tokens.get("inputTokens").and_then(Value::as_i64).unwrap_or(0);
    let output = tokens
        .get("outputTokens")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    if input == 0 && output == 0 {
        return None;
    }
    let model_raw = v
        .get("modelInfo")
        .and_then(|m| m.get("modelName").or_else(|| m.get("modelName")))
        .and_then(Value::as_str)
        .or_else(|| v.get("modelType").and_then(Value::as_str))
        .unwrap_or("")
        .to_string();
    let parsed = parse_model(&model_raw);
    let session = key.split(':').nth(1).unwrap_or("cursor").to_string();
    Some(LedgerTurn {
        ts: v.get("createdAt").and_then(Value::as_i64).unwrap_or(0) / 1000,
        harness: "cursor".into(),
        session,
        model_raw,
        model_base: parsed.base,
        effort: parsed.effort,
        provider: parsed.provider,
        input_tokens: input,
        output_tokens: output,
        confidence: "partial".into(),
        source_path: path.display().to_string(),
        ..LedgerTurn::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;

    #[test]
    fn skips_zero_token_bubbles() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("state.vscdb");
        let conn = Connection::open(&db).unwrap();
        conn.execute(
            "CREATE TABLE cursorDiskKV (key TEXT PRIMARY KEY, value TEXT)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO cursorDiskKV (key, value) VALUES (?1, ?2)",
            params![
                "bubbleId:abc",
                r#"{"tokenCount":{"inputTokens":0,"outputTokens":0}}"#
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO cursorDiskKV (key, value) VALUES (?1, ?2)",
            params![
                "bubbleId:def",
                r#"{"tokenCount":{"inputTokens":1200,"outputTokens":80},"createdAt":1700000000000}"#
            ],
        )
        .unwrap();
        let turns = parse_vscdb(&db).unwrap();
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].input_tokens, 1200);
        assert_eq!(turns[0].confidence, "partial");
    }
}
