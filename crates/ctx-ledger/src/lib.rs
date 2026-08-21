//! Parse on-disk harness transcripts into measured ledger turns.
//!
//! Never invent a model id or a price. Confidence is `measured`, `partial`,
//! or `inferred`.

mod claude;
mod codex;
mod cursor;
mod model;

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, UNIX_EPOCH};

use ctx_store::{LedgerSource, LedgerTurn, Store};
use serde_json::Value;

pub use model::{parse_model, ModelId};

#[derive(Debug, Default)]
pub struct SyncReport {
    pub inserted: u64,
    pub skipped: u64,
    pub files: u64,
    pub errors: Vec<String>,
}

impl SyncReport {
    pub fn merge(&mut self, other: SyncReport) {
        self.inserted += other.inserted;
        self.skipped += other.skipped;
        self.files += other.files;
        self.errors.extend(other.errors);
    }
}

pub fn sync_all(store: &Store) -> SyncReport {
    let mut report = SyncReport::default();
    report.merge(claude::sync(store));
    report.merge(codex::sync(store));
    report.merge(cursor::sync(store));
    report
}

/// Incremental sync, at most once per `min_interval`. Dashboard polls call this.
pub fn sync_if_due(store: &Store, min_interval: Duration) -> SyncReport {
    static LAST: OnceLock<Mutex<Instant>> = OnceLock::new();
    let slot = LAST.get_or_init(|| Mutex::new(Instant::now() - Duration::from_secs(86_400)));
    let Ok(mut last) = slot.lock() else {
        return SyncReport::default();
    };
    if last.elapsed() < min_interval {
        return SyncReport::default();
    }
    *last = Instant::now();
    drop(last);
    sync_all(store)
}

pub fn ingest_turns(store: &Store, turns: &[LedgerTurn]) -> SyncReport {
    let mut report = SyncReport {
        files: 0,
        ..SyncReport::default()
    };
    for turn in turns {
        let mut t = turn.clone();
        if t.model_base.is_empty() {
            if let Ok(model) = store.session_model(&t.session) {
                if !model.is_empty() && model != "auto" && model != "default" {
                    t.model_base = model;
                    if t.confidence == "measured" || t.confidence.is_empty() {
                        t.confidence = "inferred".into();
                    }
                }
            }
        }
        match store.insert_ledger_turn(&t) {
            Ok(true) => report.inserted += 1,
            Ok(false) => report.skipped += 1,
            Err(err) => report.errors.push(err.to_string()),
        }
    }
    report
}

struct FileDelta {
    text: String,
    extra: String,
    unchanged: bool,
}

fn read_new_text(store: &Store, path: &Path) -> Option<FileDelta> {
    let meta = std::fs::metadata(path).ok()?;
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let size = meta.len() as i64;
    let key = path.display().to_string();
    let prev = store.ledger_source(&key).ok().flatten().unwrap_or_default();
    if prev.mtime == mtime && prev.size == size && prev.offset >= size {
        return Some(FileDelta {
            text: String::new(),
            extra: prev.extra,
            unchanged: true,
        });
    }
    let offset = if size >= prev.size && prev.offset <= size {
        prev.offset
    } else {
        0
    };
    let mut file = File::open(path).ok()?;
    if offset > 0 {
        file.seek(SeekFrom::Start(offset as u64)).ok()?;
    }
    let mut text = String::new();
    file.read_to_string(&mut text).ok()?;
    Some(FileDelta {
        text,
        extra: prev.extra,
        unchanged: false,
    })
}

fn commit_source(store: &Store, path: &Path, extra: String) {
    let Ok(meta) = std::fs::metadata(path) else {
        return;
    };
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let size = meta.len() as i64;
    let _ = store.put_ledger_source(&LedgerSource {
        path: path.display().to_string(),
        mtime,
        size,
        offset: size,
        extra,
    });
}

fn json_i64(v: &Value, key: &str) -> i64 {
    v.get(key).map(value_i64).unwrap_or(0)
}

fn value_i64(v: &Value) -> i64 {
    if let Some(n) = v.as_i64() {
        return n;
    }
    if let Some(n) = v.as_u64() {
        return n as i64;
    }
    if let Some(n) = v.as_f64() {
        return n as i64;
    }
    if let Some(s) = v.as_str() {
        return s.parse().unwrap_or(0);
    }
    0
}

fn ts_secs(v: &Value) -> i64 {
    if let Some(n) = v.as_i64() {
        return millis_to_secs(n);
    }
    if let Some(n) = v.as_u64() {
        return millis_to_secs(n as i64);
    }
    if let Some(n) = v.as_f64() {
        return millis_to_secs(n as i64);
    }
    if let Some(s) = v.as_str() {
        if let Ok(n) = s.parse::<i64>() {
            return millis_to_secs(n);
        }
        return now_unix_from_rfc3339(s).unwrap_or(0);
    }
    0
}

fn millis_to_secs(n: i64) -> i64 {
    if n > 10_000_000_000 {
        n / 1000
    } else {
        n
    }
}

fn now_unix_from_rfc3339(s: &str) -> Option<i64> {
    // 2026-08-18T10:24:23.458Z or +08:00
    let s = s.trim();
    let (date, rest) = s.split_once('T')?;
    let mut d = date.split('-');
    let y: i32 = d.next()?.parse().ok()?;
    let mo: u32 = d.next()?.parse().ok()?;
    let day: u32 = d.next()?.parse().ok()?;
    let time = rest
        .split(['Z', 'z', '+'])
        .next()
        .unwrap_or(rest)
        .trim_end_matches(|c: char| c == '-' && rest.contains(':'));
    let time = if let Some(idx) = time.rfind('-') {
        if idx > 2 {
            &time[..idx]
        } else {
            time
        }
    } else {
        time
    };
    let mut t = time.split(':');
    let h: u32 = t.next()?.parse().ok()?;
    let mi: u32 = t.next()?.parse().ok()?;
    let sec_s = t.next().unwrap_or("0");
    let sec: u32 = sec_s.split('.').next()?.parse().ok()?;
    Some(unix_civil(y, mo, day, h, mi, sec))
}

fn unix_civil(y: i32, mo: u32, d: u32, h: u32, mi: u32, s: u32) -> i64 {
    let mut y = y;
    let mut m = mo as i64;
    if m <= 2 {
        y -= 1;
        m += 12;
    }
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as i64;
    let doy = (153 * (m - 3) + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era as i64 * 146097 + doe - 719468;
    days * 86400 + h as i64 * 3600 + mi as i64 * 60 + s as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc3339_parses() {
        let t = now_unix_from_rfc3339("2026-08-18T10:24:23.458Z").unwrap();
        assert!(t > 1_700_000_000, "{t}");
    }

    #[test]
    fn rfc3339_strips_offset() {
        let t = now_unix_from_rfc3339("2026-08-21T06:00:00+08:00").unwrap();
        assert_eq!(t, now_unix_from_rfc3339("2026-08-21T06:00:00Z").unwrap());
    }

    #[test]
    fn millis_epoch_divides() {
        assert_eq!(millis_to_secs(1_787_204_160_000), 1_787_204_160);
        assert_eq!(millis_to_secs(1_787_204_160), 1_787_204_160);
    }
}
