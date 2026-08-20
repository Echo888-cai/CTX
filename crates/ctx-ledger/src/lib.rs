//! Parse on-disk harness transcripts into measured ledger turns.
//!
//! Never invent a model id or a price. Confidence is `measured`, `partial`,
//! or `inferred`.

mod claude;
mod codex;
mod cursor;
mod model;

use ctx_store::{LedgerTurn, Store};

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

fn now_unix_from_rfc3339(s: &str) -> Option<i64> {
    // 2026-08-18T10:24:23.458Z
    let s = s.trim().trim_end_matches('Z');
    let (date, time) = s.split_once('T')?;
    let mut d = date.split('-');
    let y: i32 = d.next()?.parse().ok()?;
    let mo: u32 = d.next()?.parse().ok()?;
    let day: u32 = d.next()?.parse().ok()?;
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
}
