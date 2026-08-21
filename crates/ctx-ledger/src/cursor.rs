use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use ctx_store::{LedgerSource, LedgerTurn, Store};
use rusqlite::{Connection, OpenFlags};
use serde_json::{json, Value};

use crate::{ingest_turns, json_i64, parse_model, ts_secs, value_i64, SyncReport};

pub fn sync(store: &Store) -> SyncReport {
    let mut report = SyncReport::default();
    for db in cursor_vscdb_paths() {
        if !db.is_file() {
            continue;
        }
        match parse_vscdb(store, &db) {
            Ok(turns) => {
                let mut inner = ingest_turns(store, &turns);
                inner.files = 1;
                report.merge(inner);
            }
            Err(err) => {
                report.files += 1;
                report.errors.push(err);
            }
        }
    }
    report
}

fn cursor_vscdb_paths() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(home) = dirs::home_dir() {
        out.push(
            home.join("Library")
                .join("Application Support")
                .join("Cursor")
                .join("User")
                .join("globalStorage")
                .join("state.vscdb"),
        );
        out.push(
            home.join(".config")
                .join("Cursor")
                .join("User")
                .join("globalStorage")
                .join("state.vscdb"),
        );
        out.push(
            home.join(".config")
                .join("cursor")
                .join("User")
                .join("globalStorage")
                .join("state.vscdb"),
        );
    }
    if let Ok(appdata) = std::env::var("APPDATA") {
        out.push(
            PathBuf::from(appdata)
                .join("Cursor")
                .join("User")
                .join("globalStorage")
                .join("state.vscdb"),
        );
    }
    out.sort();
    out.dedup();
    out
}

pub fn parse_vscdb(store: &Store, path: &PathBuf) -> Result<Vec<LedgerTurn>, String> {
    let key = path.display().to_string();
    let prev = store.ledger_source(&key).unwrap_or(None).unwrap_or_default();
    let meta = std::fs::metadata(path).map_err(|e| e.to_string())?;
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let size = meta.len() as i64;
    if prev.mtime == mtime && prev.size == size && !prev.extra.is_empty() {
        return Ok(Vec::new());
    }

    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|e| e.to_string())?;
    let mut watermarks = watermark_map(&prev.extra);
    let mut turns = parse_composers(&conn, path, &mut watermarks)?;
    if let Ok(bubbles) = parse_bubbles(&conn, path) {
        turns.extend(bubbles);
    }

    let extra = json!({ "c": watermarks }).to_string();
    let _ = store.put_ledger_source(&LedgerSource {
        path: key,
        mtime,
        size,
        offset: size,
        extra,
    });
    Ok(turns)
}

fn parse_composers(
    conn: &Connection,
    path: &Path,
    watermarks: &mut BTreeMap<String, (i64, i64)>,
) -> Result<Vec<LedgerTurn>, String> {
    let mut stmt = conn
        .prepare("SELECT key, value FROM cursorDiskKV WHERE key LIKE 'composerData:%'")
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
        if let Some(turn) = from_composer(&key, &v, path, watermarks) {
            turns.push(turn);
        }
    }
    Ok(turns)
}

fn parse_bubbles(conn: &Connection, path: &Path) -> Result<Vec<LedgerTurn>, String> {
    // Legacy Cursor wrote per-bubble tokenCount. Current builds store zeros.
    // Only pull rows that actually have tokens so a 50k-bubble db stays cheap.
    let mut stmt = conn
        .prepare(
            "SELECT key, value FROM cursorDiskKV
             WHERE key LIKE 'bubbleId:%'
               AND (
                    json_extract(value, '$.tokenCount.inputTokens') > 0
                 OR json_extract(value, '$.tokenCount.outputTokens') > 0
                 OR json_extract(value, '$.tokenCount.cacheReadTokens') > 0
                 OR json_extract(value, '$.tokenCount.cacheReadInputTokens') > 0
               )
             LIMIT 4000",
        )
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

fn from_composer(
    key: &str,
    v: &Value,
    path: &Path,
    watermarks: &mut BTreeMap<String, (i64, i64)>,
) -> Option<LedgerTurn> {
    let session = key.split(':').nth(1).unwrap_or("cursor").to_string();
    let updated = ts_secs(v.get("lastUpdatedAt").unwrap_or(&Value::Null));
    let created = ts_secs(v.get("createdAt").unwrap_or(&Value::Null));
    let ts = if updated > 0 { updated } else { created };
    if ts <= 0 {
        return None;
    }
    let tokens = json_i64(v, "contextTokensUsed").max(json_i64(
        v.get("promptTokenBreakdown").unwrap_or(&Value::Null),
        "totalUsedTokens",
    ));
    if tokens <= 0 {
        return None;
    }
    let prev = watermarks.get(&session).copied();
    if let Some((prev_u, prev_t)) = prev {
        if prev_u == updated && prev_t == tokens {
            return None;
        }
        if prev_t == tokens {
            watermarks.insert(session, (updated, tokens));
            return None;
        }
    }
    let (cache_read, cache_write, input) = inferred_cache(prev.map(|(_, t)| t), tokens);
    watermarks.insert(session.clone(), (updated, tokens));
    let model_raw = composer_model(v);
    let parsed = parse_model(&model_raw);
    Some(LedgerTurn {
        ts,
        harness: "cursor".into(),
        session,
        model_raw,
        model_base: parsed.base,
        effort: parsed.effort,
        provider: parsed.provider,
        input_tokens: input,
        cache_read_tokens: cache_read,
        cache_write_5m: cache_write,
        context_window: json_i64(v, "contextTokenLimit"),
        confidence: "inferred".into(),
        source_path: path.display().to_string(),
        ..LedgerTurn::default()
    })
}

/// Cursor's local DB does not store prompt-cache usage. Approximate it from
/// the context-window snapshot: a growing conversation re-sends the previous
/// window (cache read) plus the delta (fresh + cache write). A shrink is a
/// compact / new epoch.
fn inferred_cache(prev: Option<i64>, curr: i64) -> (i64, i64, i64) {
    let Some(prev) = prev.filter(|p| *p > 0) else {
        return (0, curr, curr);
    };
    if curr < (prev as f64 * 0.85) as i64 {
        return (0, curr, curr);
    }
    let cache_read = prev.min(curr);
    let fresh = (curr - cache_read).max(0);
    let cache_write = (curr - prev).max(0);
    (cache_read, cache_write, fresh)
}

fn composer_model(v: &Value) -> String {
    v.get("modelConfig")
        .and_then(|m| {
            m.get("modelName")
                .and_then(Value::as_str)
                .or_else(|| {
                    m.get("selectedModels")
                        .and_then(Value::as_array)
                        .and_then(|a| a.first())
                        .and_then(|m| m.get("modelId"))
                        .and_then(Value::as_str)
                })
        })
        .or_else(|| v.get("modelName").and_then(Value::as_str))
        .unwrap_or("")
        .to_string()
}

fn from_bubble(key: &str, v: &Value, path: &Path) -> Option<LedgerTurn> {
    let tokens = v.get("tokenCount")?;
    let input = cache_field(tokens, &["inputTokens", "input_tokens"]);
    let output = cache_field(tokens, &["outputTokens", "output_tokens"]);
    let cache_read = cache_field(
        tokens,
        &[
            "cacheReadTokens",
            "cacheReadInputTokens",
            "cachedTokens",
            "cache_read_input_tokens",
        ],
    );
    let cache_write = cache_field(
        tokens,
        &[
            "cacheWriteTokens",
            "cacheCreationTokens",
            "cache_creation_input_tokens",
        ],
    );
    if input == 0 && output == 0 && cache_read == 0 {
        return None;
    }
    let model_raw = v
        .get("modelInfo")
        .and_then(|m| m.get("modelName"))
        .and_then(Value::as_str)
        .or_else(|| v.get("modelType").and_then(Value::as_str))
        .unwrap_or("")
        .to_string();
    let parsed = parse_model(&model_raw);
    let session = key.split(':').nth(1).unwrap_or("cursor").to_string();
    let ts = ts_secs(v.get("createdAt").unwrap_or(&Value::Null));
    Some(LedgerTurn {
        ts,
        harness: "cursor".into(),
        session,
        model_raw,
        model_base: parsed.base,
        effort: parsed.effort,
        provider: parsed.provider,
        input_tokens: input,
        output_tokens: output,
        cache_read_tokens: cache_read,
        cache_write_5m: cache_write,
        confidence: if cache_read > 0 { "partial" } else { "partial" }.into(),
        source_path: path.display().to_string(),
        ..LedgerTurn::default()
    })
}

fn cache_field(v: &Value, names: &[&str]) -> i64 {
    for name in names {
        let n = json_i64(v, name);
        if n > 0 {
            return n;
        }
    }
    0
}

fn watermark_map(extra: &str) -> BTreeMap<String, (i64, i64)> {
    let Ok(v) = serde_json::from_str::<Value>(extra) else {
        return BTreeMap::new();
    };
    let Some(obj) = v.get("c").and_then(Value::as_object) else {
        return BTreeMap::new();
    };
    let mut out = BTreeMap::new();
    for (id, row) in obj {
        if let Some(arr) = row.as_array() {
            if arr.len() >= 2 {
                out.insert(id.clone(), (value_i64(&arr[0]), value_i64(&arr[1])));
                continue;
            }
        }
        if let Some(o) = row.as_object() {
            out.insert(
                id.clone(),
                (
                    o.get("u").map(value_i64).unwrap_or(0),
                    o.get("t").map(value_i64).unwrap_or(0),
                ),
            );
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use ctx_store::CtxPaths;
    use rusqlite::params;

    fn db_with(rows: &[(&str, &str)]) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("state.vscdb");
        let conn = Connection::open(&db).unwrap();
        conn.execute(
            "CREATE TABLE cursorDiskKV (key TEXT PRIMARY KEY, value TEXT)",
            [],
        )
        .unwrap();
        for (k, v) in rows {
            conn.execute(
                "INSERT INTO cursorDiskKV (key, value) VALUES (?1, ?2)",
                params![k, v],
            )
            .unwrap();
        }
        (dir, db)
    }

    #[test]
    fn skips_zero_token_bubbles() {
        let store_dir = tempfile::tempdir().unwrap();
        let store = Store::open(CtxPaths::from_root(store_dir.path().to_path_buf())).unwrap();
        let (_dir, db) = db_with(&[
            (
                "bubbleId:abc:1",
                r#"{"tokenCount":{"inputTokens":0,"outputTokens":0},"createdAt":"2026-08-21T00:00:00Z"}"#,
            ),
            (
                "bubbleId:def:1",
                r#"{"tokenCount":{"inputTokens":1200,"outputTokens":80},"createdAt":1700000000000}"#,
            ),
        ]);
        let turns = parse_vscdb(&store, &db).unwrap();
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].input_tokens, 1200);
        assert_eq!(turns[0].ts, 1_700_000_000);
        assert_eq!(turns[0].confidence, "partial");
    }

    #[test]
    fn infers_cache_from_growing_composer_window() {
        let store_dir = tempfile::tempdir().unwrap();
        let store = Store::open(CtxPaths::from_root(store_dir.path().to_path_buf())).unwrap();
        let composer = r#"{
            "composerId":"abc",
            "lastUpdatedAt":1787241600000,
            "createdAt":1787240000000,
            "contextTokensUsed":100000,
            "contextTokenLimit":256000,
            "modelConfig":{"modelName":"grok-4.6"}
        }"#;
        let (_dir, db) = db_with(&[("composerData:abc", composer)]);
        let first = parse_vscdb(&store, &db).unwrap();
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].cache_read_tokens, 0);
        assert_eq!(first[0].input_tokens, 100000);
        assert_eq!(first[0].confidence, "inferred");
        assert_eq!(first[0].model_base, "grok-4.6");

        let grown = r#"{
            "composerId":"abc",
            "lastUpdatedAt":1787241660000,
            "createdAt":1787240000000,
            "contextTokensUsed":110000,
            "contextTokenLimit":256000,
            "modelConfig":{"modelName":"grok-4.6"}
        }"#;
        let conn = Connection::open(&db).unwrap();
        conn.execute(
            "UPDATE cursorDiskKV SET value = ?1 WHERE key = 'composerData:abc'",
            params![grown],
        )
        .unwrap();
        // bump mtime by rewriting extra empty so parse does not short-circuit
        store
            .put_ledger_source(&LedgerSource {
                path: db.display().to_string(),
                mtime: 0,
                size: 0,
                offset: 0,
                extra: store
                    .ledger_source(&db.display().to_string())
                    .unwrap()
                    .unwrap()
                    .extra,
            })
            .unwrap();
        let second = parse_vscdb(&store, &db).unwrap();
        assert_eq!(second.len(), 1);
        assert_eq!(second[0].cache_read_tokens, 100000);
        assert_eq!(second[0].input_tokens, 10000);
        assert_eq!(second[0].cache_write_5m, 10000);
    }

    #[test]
    fn inferred_cache_treats_shrink_as_new_epoch() {
        assert_eq!(inferred_cache(Some(200_000), 80_000), (0, 80_000, 80_000));
        assert_eq!(inferred_cache(None, 50_000), (0, 50_000, 50_000));
        assert_eq!(
            inferred_cache(Some(100_000), 110_000),
            (100_000, 10_000, 10_000)
        );
    }
}
