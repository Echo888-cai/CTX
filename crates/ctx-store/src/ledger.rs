//! Measured provider usage, cache epochs, and copy-on-write overlays.

use rusqlite::params;
use serde::Serialize;

use super::{now_secs, Result, Store};

#[derive(Debug, Clone, Serialize, Default)]
pub struct LedgerTurn {
    pub ts: i64,
    pub harness: String,
    pub session: String,
    pub cwd: String,
    pub model_raw: String,
    pub model_base: String,
    pub effort: String,
    pub provider: String,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_write_5m: i64,
    pub cache_write_1h: i64,
    pub reasoning_tokens: i64,
    pub context_window: i64,
    pub is_compaction: bool,
    pub quota_used_pct: Option<f64>,
    pub plan_type: String,
    pub confidence: String,
    pub source_path: String,
    pub resets_at: String,
}

impl LedgerTurn {
    pub fn uncached_input(&self) -> i64 {
        if self.harness == "codex" || self.provider == "openai" {
            (self.input_tokens - self.cache_read_tokens).max(0)
        } else {
            self.input_tokens
        }
    }

    pub fn cache_hit_rate(&self) -> f64 {
        let denom = (self.uncached_input() + self.cache_read_tokens).max(0) as f64;
        if denom <= 0.0 {
            return 0.0;
        }
        self.cache_read_tokens as f64 / denom
    }
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct CacheTotals {
    pub turns: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub reasoning_tokens: u64,
    pub compact_events: u64,
    pub quota_used_pct: Option<f64>,
    pub plan_type: String,
}

impl CacheTotals {
    pub fn hit_rate(&self) -> f64 {
        let denom = self.input_tokens.saturating_add(self.cache_read_tokens);
        if denom == 0 {
            return 0.0;
        }
        self.cache_read_tokens as f64 / denom as f64
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct EpochRow {
    pub session_id: String,
    pub epoch: i64,
    pub model: String,
    pub thinking: String,
    pub tools_hash: String,
    pub system_hash: String,
    pub workspace_snapshot: String,
    pub prefix_hash: String,
    pub started_at: i64,
    pub ended_at: Option<i64>,
    pub rotate_reason: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct OverlayRow {
    pub session_id: String,
    pub epoch: i64,
    pub seq: i64,
    pub path: String,
    pub prev_hash: String,
    pub new_hash: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct WorkspaceSnapshotRow {
    pub id: String,
    pub created_at: i64,
    pub file_count: i64,
    pub manifest: String,
}

impl Store {
    /// Insert a measured turn. Returns false when the unique key already exists.
    pub fn insert_ledger_turn(&self, turn: &LedgerTurn) -> Result<bool> {
        let conn = self.lock();
        let n = conn.execute(
            "INSERT OR IGNORE INTO ledger_turns (
                ts, harness, session, cwd, model_raw, model_base, effort, provider,
                input_tokens, output_tokens, cache_read_tokens, cache_write_5m, cache_write_1h,
                reasoning_tokens, context_window, is_compaction, quota_used_pct, plan_type,
                confidence, source_path, resets_at
             ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21)",
            params![
                turn.ts,
                turn.harness,
                turn.session,
                turn.cwd,
                turn.model_raw,
                turn.model_base,
                turn.effort,
                turn.provider,
                turn.input_tokens,
                turn.output_tokens,
                turn.cache_read_tokens,
                turn.cache_write_5m,
                turn.cache_write_1h,
                turn.reasoning_tokens,
                turn.context_window,
                turn.is_compaction as i64,
                turn.quota_used_pct,
                turn.plan_type,
                turn.confidence,
                turn.source_path,
                turn.resets_at,
            ],
        )?;
        Ok(n > 0)
    }

    pub fn ledger_since(&self, since: i64) -> Result<Vec<LedgerTurn>> {
        let conn = self.reader();
        let mut stmt = conn.prepare(
            "SELECT ts, harness, session, cwd, model_raw, model_base, effort, provider,
                    input_tokens, output_tokens, cache_read_tokens, cache_write_5m, cache_write_1h,
                    reasoning_tokens, context_window, is_compaction, quota_used_pct, plan_type,
                    confidence, source_path, resets_at
             FROM ledger_turns WHERE ts >= ?1 ORDER BY ts ASC",
        )?;
        let rows = stmt.query_map([since], map_turn)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    pub fn ledger_totals_since(&self, since: i64) -> Result<CacheTotals> {
        let conn = self.reader();
        conn.query_row(
            "SELECT COUNT(*),
                    COALESCE(SUM(input_tokens), 0),
                    COALESCE(SUM(output_tokens), 0),
                    COALESCE(SUM(cache_read_tokens), 0),
                    COALESCE(SUM(cache_write_5m + cache_write_1h), 0),
                    COALESCE(SUM(reasoning_tokens), 0),
                    COALESCE(SUM(is_compaction), 0),
                    MAX(quota_used_pct),
                    COALESCE((SELECT plan_type FROM ledger_turns WHERE ts >= ?1 AND plan_type != '' ORDER BY ts DESC LIMIT 1), '')
             FROM ledger_turns WHERE ts >= ?1",
            [since],
            |r| {
                Ok(CacheTotals {
                    turns: r.get::<_, i64>(0)? as u64,
                    input_tokens: r.get::<_, i64>(1)? as u64,
                    output_tokens: r.get::<_, i64>(2)? as u64,
                    cache_read_tokens: r.get::<_, i64>(3)? as u64,
                    cache_write_tokens: r.get::<_, i64>(4)? as u64,
                    reasoning_tokens: r.get::<_, i64>(5)? as u64,
                    compact_events: r.get::<_, i64>(6)? as u64,
                    quota_used_pct: r.get(7)?,
                    plan_type: r.get(8)?,
                })
            },
        )
        .map_err(Into::into)
    }

    pub fn last_ledger_turn(&self, session: &str) -> Result<Option<LedgerTurn>> {
        let conn = self.reader();
        let row = conn.query_row(
            "SELECT ts, harness, session, cwd, model_raw, model_base, effort, provider,
                    input_tokens, output_tokens, cache_read_tokens, cache_write_5m, cache_write_1h,
                    reasoning_tokens, context_window, is_compaction, quota_used_pct, plan_type,
                    confidence, source_path, resets_at
             FROM ledger_turns WHERE session = ?1 ORDER BY ts DESC LIMIT 1",
            [session],
            map_turn,
        );
        match row {
            Ok(t) => Ok(Some(t)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn current_epoch(&self, session: &str) -> Result<Option<EpochRow>> {
        let conn = self.reader();
        let row = conn.query_row(
            "SELECT session_id, epoch, model, thinking, tools_hash, system_hash,
                    workspace_snapshot, prefix_hash, started_at, ended_at, rotate_reason
             FROM epochs WHERE session_id = ?1 AND ended_at IS NULL
             ORDER BY epoch DESC LIMIT 1",
            [session],
            map_epoch,
        );
        match row {
            Ok(e) => Ok(Some(e)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Open epoch 1, or keep the current epoch when the prefix is unchanged.
    pub fn ensure_epoch(
        &self,
        session: &str,
        model: &str,
        thinking: &str,
        tools_hash: &str,
        system_hash: &str,
        workspace_snapshot: &str,
        prefix_hash: &str,
    ) -> Result<EpochRow> {
        if let Some(cur) = self.current_epoch(session)? {
            if cur.prefix_hash == prefix_hash && cur.model == model && cur.thinking == thinking {
                return Ok(cur);
            }
            return self.rotate_epoch(
                session,
                &cur,
                model,
                thinking,
                tools_hash,
                system_hash,
                workspace_snapshot,
                prefix_hash,
                rotate_reason(&cur, model, thinking, prefix_hash),
            );
        }
        let row = EpochRow {
            session_id: session.to_string(),
            epoch: 1,
            model: model.to_string(),
            thinking: thinking.to_string(),
            tools_hash: tools_hash.to_string(),
            system_hash: system_hash.to_string(),
            workspace_snapshot: workspace_snapshot.to_string(),
            prefix_hash: prefix_hash.to_string(),
            started_at: now_secs(),
            ended_at: None,
            rotate_reason: String::new(),
        };
        self.insert_epoch(&row)?;
        Ok(row)
    }

    pub fn rotate_epoch(
        &self,
        session: &str,
        previous: &EpochRow,
        model: &str,
        thinking: &str,
        tools_hash: &str,
        system_hash: &str,
        workspace_snapshot: &str,
        prefix_hash: &str,
        reason: String,
    ) -> Result<EpochRow> {
        let now = now_secs();
        {
            let conn = self.lock();
            conn.execute(
                "UPDATE epochs SET ended_at = ?1, rotate_reason = ?2
                 WHERE session_id = ?3 AND epoch = ?4",
                params![now, reason, session, previous.epoch],
            )?;
        }
        let row = EpochRow {
            session_id: session.to_string(),
            epoch: previous.epoch + 1,
            model: model.to_string(),
            thinking: thinking.to_string(),
            tools_hash: tools_hash.to_string(),
            system_hash: system_hash.to_string(),
            workspace_snapshot: workspace_snapshot.to_string(),
            prefix_hash: prefix_hash.to_string(),
            started_at: now,
            ended_at: None,
            rotate_reason: String::new(),
        };
        self.insert_epoch(&row)?;
        Ok(row)
    }

    fn insert_epoch(&self, row: &EpochRow) -> Result<()> {
        let conn = self.lock();
        conn.execute(
            "INSERT OR REPLACE INTO epochs (
                session_id, epoch, model, thinking, tools_hash, system_hash,
                workspace_snapshot, prefix_hash, started_at, ended_at, rotate_reason
             ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
            params![
                row.session_id,
                row.epoch,
                row.model,
                row.thinking,
                row.tools_hash,
                row.system_hash,
                row.workspace_snapshot,
                row.prefix_hash,
                row.started_at,
                row.ended_at,
                row.rotate_reason,
            ],
        )?;
        Ok(())
    }

    pub fn put_workspace_snapshot(
        &self,
        id: &str,
        file_count: i64,
        manifest: &str,
    ) -> Result<()> {
        let conn = self.lock();
        conn.execute(
            "INSERT OR IGNORE INTO workspace_snapshots (id, created_at, file_count, manifest)
             VALUES (?1, ?2, ?3, ?4)",
            params![id, now_secs(), file_count, manifest],
        )?;
        Ok(())
    }

    pub fn workspace_snapshot(&self, id: &str) -> Result<Option<WorkspaceSnapshotRow>> {
        let conn = self.reader();
        let row = conn.query_row(
            "SELECT id, created_at, file_count, manifest FROM workspace_snapshots WHERE id = ?1",
            [id],
            |r| {
                Ok(WorkspaceSnapshotRow {
                    id: r.get(0)?,
                    created_at: r.get(1)?,
                    file_count: r.get(2)?,
                    manifest: r.get(3)?,
                })
            },
        );
        match row {
            Ok(s) => Ok(Some(s)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn push_overlay(
        &self,
        session: &str,
        path: &str,
        prev_hash: &str,
        new_hash: &str,
    ) -> Result<OverlayRow> {
        let epoch = self
            .current_epoch(session)?
            .map(|e| e.epoch)
            .unwrap_or(1);
        let conn = self.lock();
        let seq: i64 = conn
            .query_row(
                "SELECT COALESCE(MAX(seq), 0) FROM overlays WHERE session_id = ?1 AND epoch = ?2",
                params![session, epoch],
                |r| r.get(0),
            )
            .unwrap_or(0)
            + 1;
        let created = now_secs();
        conn.execute(
            "INSERT INTO overlays (session_id, epoch, seq, path, prev_hash, new_hash, created_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7)",
            params![session, epoch, seq, path, prev_hash, new_hash, created],
        )?;
        Ok(OverlayRow {
            session_id: session.to_string(),
            epoch,
            seq,
            path: path.to_string(),
            prev_hash: prev_hash.to_string(),
            new_hash: new_hash.to_string(),
            created_at: created,
        })
    }

    pub fn overlays_for(&self, session: &str, epoch: i64) -> Result<Vec<OverlayRow>> {
        let conn = self.reader();
        let mut stmt = conn.prepare(
            "SELECT session_id, epoch, seq, path, prev_hash, new_hash, created_at
             FROM overlays WHERE session_id = ?1 AND epoch = ?2 ORDER BY seq ASC",
        )?;
        let rows = stmt.query_map(params![session, epoch], |r| {
            Ok(OverlayRow {
                session_id: r.get(0)?,
                epoch: r.get(1)?,
                seq: r.get(2)?,
                path: r.get(3)?,
                prev_hash: r.get(4)?,
                new_hash: r.get(5)?,
                created_at: r.get(6)?,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    pub fn epoch_count(&self) -> Result<u64> {
        let conn = self.reader();
        let n: i64 = conn.query_row("SELECT COUNT(*) FROM epochs", [], |r| r.get(0))?;
        Ok(n as u64)
    }

    pub fn overlay_count(&self) -> Result<u64> {
        let conn = self.reader();
        let n: i64 = conn.query_row("SELECT COUNT(*) FROM overlays", [], |r| r.get(0))?;
        Ok(n as u64)
    }

    pub fn refetch_totals_since(&self, since: i64) -> Result<(u64, u64)> {
        let conn = self.reader();
        conn.query_row(
            "SELECT COALESCE(SUM(refetch_count), 0), COALESCE(SUM(refetched_tokens), 0)
             FROM observations WHERE created_at >= ?1",
            [since],
            |r| Ok((r.get::<_, i64>(0)? as u64, r.get::<_, i64>(1)? as u64)),
        )
        .map_err(Into::into)
    }

    pub fn push_journal(&self, session: &str, kind: &str, body: &str) -> Result<()> {
        let epoch = self.current_epoch(session)?.map(|e| e.epoch).unwrap_or(1);
        let conn = self.lock();
        let seq: i64 = conn
            .query_row(
                "SELECT COALESCE(MAX(seq), 0) FROM journal WHERE session_id = ?1 AND epoch = ?2",
                params![session, epoch],
                |r| r.get(0),
            )
            .unwrap_or(0)
            + 1;
        conn.execute(
            "INSERT INTO journal (session_id, epoch, seq, kind, body, created_at)
             VALUES (?1,?2,?3,?4,?5,?6)",
            params![session, epoch, seq, kind, body, now_secs()],
        )?;
        Ok(())
    }

    pub fn journal_text(&self, session: &str, epoch: i64) -> Result<String> {
        let conn = self.reader();
        let mut stmt = conn.prepare(
            "SELECT kind, body FROM journal WHERE session_id = ?1 AND epoch = ?2 ORDER BY seq ASC",
        )?;
        let rows = stmt.query_map(params![session, epoch], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?;
        let mut out = String::new();
        for row in rows {
            let (kind, body) = row?;
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(&kind);
            if !body.is_empty() {
                out.push(' ');
                out.push_str(&body);
            }
        }
        Ok(out)
    }

    pub fn put_capability(&self, handle: &str, name: &str, description: &str, schema: &str) -> Result<()> {
        let conn = self.lock();
        conn.execute(
            "INSERT INTO capabilities (handle, name, description, schema_json, created_at)
             VALUES (?1,?2,?3,?4,?5)
             ON CONFLICT(handle) DO UPDATE SET
                name = excluded.name,
                description = excluded.description,
                schema_json = excluded.schema_json",
            params![handle, name, description, schema, now_secs()],
        )?;
        Ok(())
    }

    pub fn get_capability(&self, handle: &str) -> Result<Option<(String, String, String)>> {
        let conn = self.reader();
        let row = conn.query_row(
            "SELECT name, description, schema_json FROM capabilities WHERE handle = ?1",
            [handle],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        );
        match row {
            Ok(v) => Ok(Some(v)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn search_capabilities(&self, query: &str, limit: usize) -> Result<Vec<(String, String, String)>> {
        let q = query.trim().to_ascii_lowercase();
        if q.len() < 2 {
            return Ok(Vec::new());
        }
        let pat = format!("%{q}%");
        let conn = self.reader();
        let mut stmt = conn.prepare(
            "SELECT handle, name, description FROM capabilities
             WHERE lower(handle) LIKE ?1 OR lower(name) LIKE ?1 OR lower(description) LIKE ?1
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![pat, limit as i64], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    pub fn session_model(&self, id: &str) -> Result<String> {
        let conn = self.reader();
        let row = conn.query_row(
            "SELECT COALESCE(model, '') FROM sessions WHERE id = ?1",
            [id],
            |r| r.get::<_, String>(0),
        );
        match row {
            Ok(s) => Ok(s),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(String::new()),
            Err(e) => Err(e.into()),
        }
    }

    pub fn totals_since_shadow(&self, since_unix: i64, shadow: bool) -> Result<crate::TokenTotals> {
        let conn = self.reader();
        conn.query_row(
            "SELECT
                COALESCE(SUM(raw_tokens), 0),
                COALESCE(SUM(delivered_tokens), 0),
                COALESCE(SUM(avoided_tokens), 0),
                COALESCE(SUM(refetched_tokens), 0)
             FROM observations WHERE created_at >= ?1 AND shadow = ?2",
            params![since_unix, if shadow { 1i64 } else { 0i64 }],
            |r| {
                Ok(crate::TokenTotals {
                    raw: r.get::<_, i64>(0)? as u64,
                    delivered: r.get::<_, i64>(1)? as u64,
                    avoided: r.get::<_, i64>(2)? as u64,
                    refetched: r.get::<_, i64>(3).unwrap_or(0) as u64,
                })
            },
        )
        .map_err(Into::into)
    }

    /// Sessions in the window with no fail/error referenced bit — a coarse "task passed".
    pub fn clean_sessions_since(&self, since: i64) -> Result<u64> {
        let conn = self.reader();
        let n: i64 = conn.query_row(
            "SELECT COUNT(*) FROM sessions s
             WHERE s.started_at >= ?1
               AND NOT EXISTS (
                   SELECT 1 FROM observations o
                   WHERE o.session_id = s.id AND o.referenced = 1
               )",
            [since],
            |r| r.get(0),
        )?;
        Ok(n as u64)
    }
}

fn rotate_reason(cur: &EpochRow, model: &str, thinking: &str, prefix_hash: &str) -> String {
    if cur.model != model {
        return "model-switch".into();
    }
    if cur.thinking != thinking {
        return "thinking-level".into();
    }
    if cur.prefix_hash != prefix_hash {
        return "prefix-drift".into();
    }
    "rotate".into()
}

fn map_turn(r: &rusqlite::Row<'_>) -> rusqlite::Result<LedgerTurn> {
    Ok(LedgerTurn {
        ts: r.get(0)?,
        harness: r.get(1)?,
        session: r.get(2)?,
        cwd: r.get(3)?,
        model_raw: r.get(4)?,
        model_base: r.get(5)?,
        effort: r.get(6)?,
        provider: r.get(7)?,
        input_tokens: r.get(8)?,
        output_tokens: r.get(9)?,
        cache_read_tokens: r.get(10)?,
        cache_write_5m: r.get(11)?,
        cache_write_1h: r.get(12)?,
        reasoning_tokens: r.get(13)?,
        context_window: r.get(14)?,
        is_compaction: r.get::<_, i64>(15)? != 0,
        quota_used_pct: r.get(16)?,
        plan_type: r.get(17)?,
        confidence: r.get(18)?,
        source_path: r.get(19)?,
        resets_at: r.get::<_, String>(20).unwrap_or_default(),
    })
}

fn map_epoch(r: &rusqlite::Row<'_>) -> rusqlite::Result<EpochRow> {
    Ok(EpochRow {
        session_id: r.get(0)?,
        epoch: r.get(1)?,
        model: r.get(2)?,
        thinking: r.get(3)?,
        tools_hash: r.get(4)?,
        system_hash: r.get(5)?,
        workspace_snapshot: r.get(6)?,
        prefix_hash: r.get(7)?,
        started_at: r.get(8)?,
        ended_at: r.get(9)?,
        rotate_reason: r.get(10)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CtxPaths;

    #[test]
    fn ledger_turn_is_deduped() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(CtxPaths::from_root(dir.path().to_path_buf())).unwrap();
        let mut t = LedgerTurn {
            ts: 1_700_000_000,
            harness: "claude-code".into(),
            session: "s".into(),
            input_tokens: 100,
            cache_read_tokens: 80,
            confidence: "measured".into(),
            ..LedgerTurn::default()
        };
        assert!(store.insert_ledger_turn(&t).unwrap());
        assert!(!store.insert_ledger_turn(&t).unwrap());
        t.ts = 1_700_000_001;
        assert!(store.insert_ledger_turn(&t).unwrap());
        let tot = store.ledger_totals_since(0).unwrap();
        assert_eq!(tot.turns, 2);
        assert_eq!(tot.cache_read_tokens, 160);
    }

    #[test]
    fn epoch_rotates_on_model_change() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(CtxPaths::from_root(dir.path().to_path_buf())).unwrap();
        let a = store
            .ensure_epoch("s", "claude-sonnet-4", "", "t", "sys", "snap", "p1")
            .unwrap();
        assert_eq!(a.epoch, 1);
        let b = store
            .ensure_epoch("s", "claude-opus-4", "", "t", "sys", "snap", "p1")
            .unwrap();
        assert_eq!(b.epoch, 2);
        let again = store
            .ensure_epoch("s", "claude-opus-4", "", "t", "sys", "snap", "p1")
            .unwrap();
        assert_eq!(again.epoch, 2);
    }

    #[test]
    fn overlay_is_append_only() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(CtxPaths::from_root(dir.path().to_path_buf())).unwrap();
        store
            .ensure_epoch("s", "m", "", "", "", "", "p")
            .unwrap();
        store.push_overlay("s", "src/auth.ts", "aaa", "bbb").unwrap();
        store.push_overlay("s", "src/api.ts", "ccc", "ddd").unwrap();
        let rows = store.overlays_for("s", 1).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].seq, 1);
        assert_eq!(rows[1].path, "src/api.ts");
    }
}
