//! Local context store.
//!
//! Principle 1: raw context is immutable. Optimization never mutates stored bytes.
//! Principle 2: every virtualized payload has a `ctx://` URI that can restore the original.

mod blob;
mod db;
mod error;
mod paths;

pub use blob::{blake3_hex, normalize_hash};
pub use error::StoreError;
pub use paths::{CtxPaths, DEFAULT_HOME_ENV};

use std::sync::Mutex;
use std::time::Duration;

use rusqlite::{params, Connection};

use ctx_protocol::{CtxUri, Frame};

use crate::db::migrate;

pub type Result<T> = std::result::Result<T, StoreError>;

#[derive(Debug, Clone)]
pub struct PutBlob {
    pub hash: String,
    pub bytes: usize,
    pub compressed_bytes: usize,
}

#[derive(Debug, Clone)]
pub struct Observation {
    pub id: i64,
    pub session_id: String,
    pub event_type: String,
    pub tool_type: Option<String>,
    pub tool_name: Option<String>,
    pub uri: Option<String>,
    pub content_hash: String,
    pub raw_tokens: u32,
    pub delivered_tokens: u32,
    pub avoided_tokens: u32,
    pub optimizer: Option<String>,
    pub reasons: serde_json::Value,
    pub created_at: i64,
    pub referenced: bool,
    pub source_path: Option<String>,
    /// Last page-in time. 0 means never fetched — clock uses created_at.
    pub accessed_at: i64,
}

#[derive(Debug, Clone)]
pub struct NewObservation {
    pub session_id: String,
    pub event_type: String,
    pub tool_type: Option<String>,
    pub tool_name: Option<String>,
    pub uri: Option<String>,
    pub content_hash: String,
    pub raw_tokens: u32,
    pub delivered_tokens: u32,
    pub avoided_tokens: u32,
    pub optimizer: Option<String>,
    pub reasons: serde_json::Value,
    pub referenced: bool,
    pub source_path: Option<String>,
}

#[derive(Debug, Clone)]
pub struct FingerprintHit {
    pub hash: String,
    pub uri: Option<String>,
    pub count: i64,
    pub first_seen_at: i64,
}

#[derive(Debug, Clone)]
pub struct FileReadRecord {
    pub path: String,
    pub content_hash: String,
    pub last_uri: Option<String>,
    pub last_tokens: u32,
    pub regions: serde_json::Value,
}

#[derive(Debug, Clone)]
pub struct SessionRow {
    pub id: String,
    pub harness: String,
    pub started_at: i64,
    pub ended_at: Option<i64>,
    pub cwd: Option<String>,
}

#[derive(Debug, Clone)]
pub struct PageMeta {
    pub uri: String,
    pub hash: String,
    pub kind: String,
    pub summary: Option<String>,
    pub raw_tokens: u32,
    pub created_at: i64,
    pub task: String,
    pub harness: String,
}

pub struct RecordPage<'a> {
    pub uri: &'a CtxUri,
    pub hash: &'a str,
    pub body: &'a str,
    pub frames: &'a [Frame],
    pub raw_tokens: u32,
    pub harness: &'a str,
    pub task: &'a str,
}

#[derive(Debug, Clone)]
pub struct FtsHit {
    pub uri: String,
    pub kind: String,
    pub snippet: String,
    pub raw_tokens: u32,
}

/// A named frame hit: virtual address, not a blob.
#[derive(Debug, Clone)]
pub struct FrameHit {
    pub uri: String,
    pub name: String,
    pub kind: String,
    pub hint: String,
}

pub struct Store {
    paths: CtxPaths,
    conn: Mutex<Connection>,
}

impl Store {
    pub fn open(paths: CtxPaths) -> Result<Self> {
        paths.ensure()?;
        let conn = Connection::open(paths.db_path())?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.busy_timeout(Duration::from_millis(5_000))?;
        migrate(&conn)?;
        Ok(Self {
            paths,
            conn: Mutex::new(conn),
        })
    }

    pub fn open_default() -> Result<Self> {
        Self::open(CtxPaths::default_home()?)
    }

    pub fn paths(&self) -> &CtxPaths {
        &self.paths
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn put_bytes(&self, bytes: &[u8]) -> Result<PutBlob> {
        let hash = blake3_hex(bytes);
        let rel = blob::blob_relpath(&hash);
        let dest = self.paths.store_dir().join(rel);
        if !dest.exists() {
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let compressed = zstd::encode_all(bytes, 3)?;
            let tmp = dest.with_extension("zst.tmp");
            std::fs::write(&tmp, &compressed)?;
            std::fs::rename(&tmp, &dest)?;
            let now = now_secs();
            let conn = self.lock();
            conn.execute(
                "INSERT OR IGNORE INTO blobs (hash, bytes, compressed_bytes, created_at)
                 VALUES (?1, ?2, ?3, ?4)",
                params![hash, bytes.len() as i64, compressed.len() as i64, now],
            )?;
            return Ok(PutBlob {
                hash,
                bytes: bytes.len(),
                compressed_bytes: compressed.len(),
            });
        }
        Ok(PutBlob {
            hash,
            bytes: bytes.len(),
            compressed_bytes: std::fs::metadata(&dest)?.len() as usize,
        })
    }

    pub fn get_bytes(&self, hash: &str) -> Result<Vec<u8>> {
        let dest = self.paths.store_dir().join(blob::blob_relpath(hash));
        let compressed = std::fs::read(&dest).map_err(|_| StoreError::NotFound(hash.into()))?;
        Ok(zstd::decode_all(compressed.as_slice())?)
    }

    pub fn get_bytes_by_uri(&self, uri: &CtxUri) -> Result<Vec<u8>> {
        let hash = self.hash_for_uri(uri)?;
        self.get_bytes(&hash)
    }

    pub fn put_page(
        &self,
        uri: &CtxUri,
        hash: &str,
        summary: Option<&str>,
        raw_tokens: u32,
    ) -> Result<()> {
        let conn = self.lock();
        conn.execute(
            "INSERT OR REPLACE INTO pages (uri, hash, kind, summary, raw_tokens, created_at, task, harness)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, '', '')",
            params![
                uri.page_key(),
                hash,
                uri.kind,
                summary,
                raw_tokens,
                now_secs()
            ],
        )?;
        Ok(())
    }

    /// Page row + FTS + frame table in one transaction.
    pub fn record_page(&self, page: RecordPage<'_>) -> Result<()> {
        let key = page.uri.page_key();
        let clipped = clip_fts(page.body);
        let summary = first_signal_frame(page.frames);
        let conn = self.lock();
        let tx = conn.unchecked_transaction()?;
        tx.execute(
            "INSERT OR REPLACE INTO pages (uri, hash, kind, summary, raw_tokens, created_at, task, harness)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                &key,
                page.hash,
                &page.uri.kind,
                summary,
                page.raw_tokens,
                now_secs(),
                page.task,
                page.harness
            ],
        )?;
        if !page.body.is_empty() {
            tx.execute("DELETE FROM pages_fts WHERE uri = ?1", params![&key])?;
            tx.execute(
                "INSERT INTO pages_fts (uri, kind, body) VALUES (?1, ?2, ?3)",
                params![&key, &page.uri.kind, clipped],
            )?;
        }
        tx.execute("DELETE FROM frames WHERE uri = ?1", params![&key])?;
        for f in page.frames.iter().take(48) {
            tx.execute(
                "INSERT OR REPLACE INTO frames (uri, name, kind, start_line, end_line, hint)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    &key,
                    &f.name,
                    &f.kind,
                    f.start_line as i64,
                    f.end_line as i64,
                    &f.hint
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn hash_for_uri(&self, uri: &CtxUri) -> Result<String> {
        let conn = self.lock();
        let exact: rusqlite::Result<String> = conn.query_row(
            "SELECT hash FROM pages WHERE uri = ?1",
            params![uri.page_key()],
            |r| r.get(0),
        );
        if let Ok(hash) = exact {
            return Ok(hash);
        }
        let mut stmt = conn.prepare("SELECT hash FROM blobs WHERE hash LIKE ?1 LIMIT 2")?;
        let hashes: Vec<String> = stmt
            .query_map(params![format!("{}%", uri.id)], |r| r.get(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        match hashes.as_slice() {
            [one] => Ok(one.clone()),
            _ => Err(StoreError::NotFound(uri.page_key())),
        }
    }

    pub fn ensure_session(&self, id: &str, harness: &str, cwd: Option<&str>) -> Result<()> {
        let conn = self.lock();
        conn.execute(
            "INSERT OR IGNORE INTO sessions (id, harness, started_at, cwd)
             VALUES (?1, ?2, ?3, ?4)",
            params![id, harness, now_secs(), cwd],
        )?;
        Ok(())
    }

    pub fn session_task(&self, id: &str) -> Result<String> {
        let conn = self.lock();
        let row = conn.query_row(
            "SELECT COALESCE(task, '') FROM sessions WHERE id = ?1",
            params![id],
            |r| r.get::<_, String>(0),
        );
        match row {
            Ok(s) => Ok(s),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(String::new()),
            Err(e) => Err(e.into()),
        }
    }

    pub fn set_session_task(&self, id: &str, task: &str) -> Result<()> {
        let conn = self.lock();
        conn.execute(
            "UPDATE sessions SET task = ?1 WHERE id = ?2",
            params![task, id],
        )?;
        Ok(())
    }

    pub fn mark_remap(&self, id: &str) -> Result<()> {
        let conn = self.lock();
        conn.execute("UPDATE sessions SET remap = 1 WHERE id = ?1", params![id])?;
        Ok(())
    }

    /// True if this session needs a working-set remap (after compact). Clears the flag.
    pub fn take_remap(&self, id: &str) -> Result<bool> {
        let conn = self.lock();
        let flag: i64 = conn
            .query_row(
                "SELECT COALESCE(remap, 0) FROM sessions WHERE id = ?1",
                params![id],
                |r| r.get(0),
            )
            .unwrap_or(0);
        if flag != 0 {
            conn.execute("UPDATE sessions SET remap = 0 WHERE id = ?1", params![id])?;
        }
        Ok(flag != 0)
    }

    pub fn end_session(&self, id: &str) -> Result<()> {
        let conn = self.lock();
        conn.execute(
            "UPDATE sessions SET ended_at = ?1 WHERE id = ?2",
            params![now_secs(), id],
        )?;
        Ok(())
    }

    pub fn insert_observation(&self, obs: NewObservation) -> Result<i64> {
        let conn = self.lock();
        conn.execute(
            "INSERT INTO observations (
                session_id, event_type, tool_type, tool_name, uri, content_hash,
                raw_tokens, delivered_tokens, avoided_tokens, optimizer, reasons, created_at,
                referenced, source_path
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            params![
                obs.session_id,
                obs.event_type,
                obs.tool_type,
                obs.tool_name,
                obs.uri,
                obs.content_hash,
                obs.raw_tokens,
                obs.delivered_tokens,
                obs.avoided_tokens,
                obs.optimizer,
                obs.reasons.to_string(),
                now_secs(),
                if obs.referenced { 1i64 } else { 0i64 },
                obs.source_path
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn remember_fingerprint(
        &self,
        hash: &str,
        normalized_hash: &str,
        uri: Option<&str>,
    ) -> Result<FingerprintHit> {
        let now = now_secs();
        let conn = self.lock();
        if let Ok(hit) = conn.query_row(
            "SELECT hash, uri, count, first_seen_at FROM fingerprints WHERE hash = ?1",
            params![hash],
            map_fingerprint,
        ) {
            conn.execute(
                "UPDATE fingerprints SET last_seen_at = ?1, count = count + 1, uri = COALESCE(?2, uri)
                 WHERE hash = ?3",
                params![now, uri, hash],
            )?;
            return Ok(FingerprintHit {
                count: hit.count + 1,
                ..hit
            });
        }
        let near = match conn.query_row(
            "SELECT hash, uri, count, first_seen_at FROM fingerprints WHERE normalized_hash = ?1",
            params![normalized_hash],
            map_fingerprint,
        ) {
            Ok(hit) => Some(hit),
            Err(rusqlite::Error::QueryReturnedNoRows) => None,
            Err(e) => return Err(e.into()),
        };
        conn.execute(
            "INSERT INTO fingerprints (hash, normalized_hash, uri, first_seen_at, last_seen_at, count)
             VALUES (?1, ?2, ?3, ?4, ?4, 1)",
            params![hash, normalized_hash, uri, now],
        )?;
        if let Some(hit) = near {
            return Ok(FingerprintHit {
                count: hit.count + 1,
                uri: hit.uri.or_else(|| uri.map(str::to_string)),
                ..hit
            });
        }
        Ok(FingerprintHit {
            hash: hash.to_string(),
            uri: uri.map(str::to_string),
            count: 1,
            first_seen_at: now,
        })
    }

    pub fn lookup_fingerprint(&self, hash: &str) -> Result<Option<FingerprintHit>> {
        let conn = self.lock();
        let hit = conn.query_row(
            "SELECT hash, uri, count, first_seen_at FROM fingerprints WHERE hash = ?1",
            params![hash],
            map_fingerprint,
        );
        match hit {
            Ok(h) => Ok(Some(h)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn lookup_normalized(&self, normalized_hash: &str) -> Result<Option<FingerprintHit>> {
        let conn = self.lock();
        let hit = conn.query_row(
            "SELECT hash, uri, count, first_seen_at FROM fingerprints WHERE normalized_hash = ?1",
            params![normalized_hash],
            map_fingerprint,
        );
        match hit {
            Ok(h) => Ok(Some(h)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn get_file_read(&self, path: &str) -> Result<Option<FileReadRecord>> {
        let conn = self.lock();
        let row = conn.query_row(
            "SELECT path, content_hash, last_uri, last_tokens, regions FROM file_reads WHERE path = ?1",
            params![path],
            |r| {
                let regions: String = r.get(4)?;
                Ok(FileReadRecord {
                    path: r.get(0)?,
                    content_hash: r.get(1)?,
                    last_uri: r.get(2)?,
                    last_tokens: r.get::<_, u32>(3)?,
                    regions: serde_json::from_str(&regions).unwrap_or(serde_json::json!([])),
                })
            },
        );
        match row {
            Ok(r) => Ok(Some(r)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn upsert_file_read(&self, rec: &FileReadRecord) -> Result<()> {
        let conn = self.lock();
        conn.execute(
            "INSERT INTO file_reads (path, content_hash, last_uri, last_tokens, regions, last_seen_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(path) DO UPDATE SET
                content_hash = excluded.content_hash,
                last_uri = excluded.last_uri,
                last_tokens = excluded.last_tokens,
                regions = excluded.regions,
                last_seen_at = excluded.last_seen_at",
            params![
                rec.path,
                rec.content_hash,
                rec.last_uri,
                rec.last_tokens,
                rec.regions.to_string(),
                now_secs()
            ],
        )?;
        Ok(())
    }

    pub fn observations_since(&self, since_unix: i64) -> Result<Vec<Observation>> {
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT id, session_id, event_type, tool_type, tool_name, uri, content_hash,
                    raw_tokens, delivered_tokens, avoided_tokens, optimizer, reasons, created_at,
                    referenced, source_path, accessed_at
             FROM observations WHERE created_at >= ?1 ORDER BY created_at ASC",
        )?;
        let rows = stmt.query_map(params![since_unix], map_observation)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub fn observations_for_session(&self, session_id: &str) -> Result<Vec<Observation>> {
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT id, session_id, event_type, tool_type, tool_name, uri, content_hash,
                    raw_tokens, delivered_tokens, avoided_tokens, optimizer, reasons, created_at,
                    referenced, source_path, accessed_at
             FROM observations WHERE session_id = ?1 ORDER BY created_at ASC",
        )?;
        let rows = stmt.query_map(params![session_id], map_observation)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub fn page_count(&self) -> Result<u64> {
        let conn = self.lock();
        let n: i64 = conn.query_row("SELECT COUNT(*) FROM pages", [], |r| r.get(0))?;
        Ok(n as u64)
    }

    /// Newest pages first. Used by page-fault search.
    pub fn recent_pages(&self, limit: usize) -> Result<Vec<PageMeta>> {
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT uri, hash, kind, summary, raw_tokens, created_at,
                    COALESCE(task, ''), COALESCE(harness, '')
             FROM pages ORDER BY created_at DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit as i64], |r| {
            Ok(PageMeta {
                uri: r.get(0)?,
                hash: r.get(1)?,
                kind: r.get(2)?,
                summary: r.get(3)?,
                raw_tokens: r.get(4)?,
                created_at: r.get(5)?,
                task: r.get(6)?,
                harness: r.get(7)?,
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    /// Clock tick: paging a URI in marks it referenced and recent (WSClock).
    pub fn touch_referenced(&self, uri: &str) -> Result<u64> {
        let now = now_secs();
        let conn = self.lock();
        let n = conn.execute(
            "UPDATE observations SET referenced = 1, accessed_at = ?1 WHERE uri = ?2",
            params![now, uri],
        )?;
        Ok(n as u64)
    }

    pub fn compressed_bytes(&self) -> Result<u64> {
        let conn = self.lock();
        let n: i64 = conn.query_row(
            "SELECT COALESCE(SUM(compressed_bytes), 0) FROM blobs",
            [],
            |r| r.get(0),
        )?;
        Ok(n as u64)
    }

    /// Index a page body for FTS page-fault search.
    pub fn index_page(&self, uri: &str, kind: &str, body: &str) -> Result<()> {
        let clipped = clip_fts(body);
        let conn = self.lock();
        conn.execute("DELETE FROM pages_fts WHERE uri = ?1", params![uri])?;
        conn.execute(
            "INSERT INTO pages_fts (uri, kind, body) VALUES (?1, ?2, ?3)",
            params![uri, kind, clipped],
        )?;
        Ok(())
    }

    pub fn search_fts(&self, query: &str, limit: usize) -> Result<Vec<FtsHit>> {
        let Some(match_q) = fts_query(query) else {
            return Ok(Vec::new());
        };
        let cap = limit.clamp(1, 16) as i64;
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT f.uri, f.kind, snippet(pages_fts, 2, '', '', '…', 16),
                    COALESCE(p.raw_tokens, 0)
             FROM pages_fts f
             LEFT JOIN pages p ON p.uri = f.uri
             WHERE pages_fts MATCH ?1
             ORDER BY bm25(pages_fts)
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![match_q, cap], |r| {
            Ok(FtsHit {
                uri: r.get(0)?,
                kind: r.get(1)?,
                snippet: r.get::<_, String>(2).unwrap_or_default(),
                raw_tokens: r.get::<_, i64>(3)? as u32,
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub fn replace_frames(&self, uri: &str, frames: &[Frame]) -> Result<()> {
        let conn = self.lock();
        let tx = conn.unchecked_transaction()?;
        tx.execute("DELETE FROM frames WHERE uri = ?1", params![uri])?;
        for f in frames.iter().take(48) {
            tx.execute(
                "INSERT OR REPLACE INTO frames (uri, name, kind, start_line, end_line, hint)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    uri,
                    f.name,
                    f.kind,
                    f.start_line as i64,
                    f.end_line as i64,
                    f.hint
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn frames_for(&self, uri: &str) -> Result<Vec<Frame>> {
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT name, kind, start_line, end_line, hint FROM frames
             WHERE uri = ?1 ORDER BY start_line ASC",
        )?;
        let rows = stmt.query_map(params![uri], |r| {
            Ok(Frame {
                name: r.get(0)?,
                kind: r.get(1)?,
                start_line: r.get::<_, i64>(2)? as u32,
                end_line: r.get::<_, i64>(3)? as u32,
                hint: r.get(4)?,
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub fn find_frame(&self, uri: &str, query: &str) -> Result<Option<Frame>> {
        let q = query.trim();
        if q.is_empty() {
            return Ok(None);
        }
        let frames = self.frames_for(uri)?;
        let lower = q.to_ascii_lowercase();
        Ok(best_frame(&frames, &lower))
    }

    /// Walk the frame table: name/hint match, not a blob scan.
    pub fn search_frames(&self, query: &str, limit: usize) -> Result<Vec<FrameHit>> {
        let q = query.trim();
        if q.len() < 2 {
            return Ok(Vec::new());
        }
        let escaped = q
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_");
        let pat = format!("%{}%", escaped.to_ascii_lowercase());
        let cap = limit.clamp(1, 16) as i64;
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT uri, name, kind, hint FROM frames
             WHERE lower(name) LIKE ?1 ESCAPE '\\'
                OR lower(hint) LIKE ?1 ESCAPE '\\'
             ORDER BY start_line ASC
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![pat, cap], |r| {
            Ok(FrameHit {
                uri: r.get(0)?,
                name: r.get(1)?,
                kind: r.get(2)?,
                hint: r.get(3)?,
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    /// Previous page of the same kind in this session (for CoW). Excludes `uri`.
    pub fn last_page_for(
        &self,
        session: &str,
        kind: &str,
        except_uri: &str,
    ) -> Result<Option<(String, String)>> {
        let conn = self.lock();
        let row = conn.query_row(
            "SELECT uri, content_hash FROM observations
             WHERE session_id = ?1 AND tool_type = ?2 AND uri IS NOT NULL AND uri != ?3
             ORDER BY id DESC LIMIT 1",
            params![session, kind, except_uri],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
        );
        match row {
            Ok(v) => Ok(Some(v)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn totals_since(&self, since_unix: i64) -> Result<TokenTotals> {
        let conn = self.lock();
        conn.query_row(
            "SELECT
                COALESCE(SUM(raw_tokens), 0),
                COALESCE(SUM(delivered_tokens), 0),
                COALESCE(SUM(avoided_tokens), 0)
             FROM observations WHERE created_at >= ?1",
            params![since_unix],
            |r| {
                Ok(TokenTotals {
                    raw: r.get::<_, i64>(0)? as u64,
                    delivered: r.get::<_, i64>(1)? as u64,
                    avoided: r.get::<_, i64>(2)? as u64,
                })
            },
        )
        .map_err(Into::into)
    }

    pub fn totals_by_harness_since(&self, since_unix: i64) -> Result<Vec<(String, TokenTotals)>> {
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT s.harness,
                    COALESCE(SUM(o.raw_tokens), 0),
                    COALESCE(SUM(o.delivered_tokens), 0),
                    COALESCE(SUM(o.avoided_tokens), 0)
             FROM observations o
             JOIN sessions s ON s.id = o.session_id
             WHERE o.created_at >= ?1
             GROUP BY s.harness",
        )?;
        let rows = stmt.query_map(params![since_unix], |r| {
            Ok((
                r.get::<_, String>(0)?,
                TokenTotals {
                    raw: r.get::<_, i64>(1)? as u64,
                    delivered: r.get::<_, i64>(2)? as u64,
                    avoided: r.get::<_, i64>(3)? as u64,
                },
            ))
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub fn reason_breakdown_since(&self, since_unix: i64) -> Result<Vec<(String, u64)>> {
        let conn = self.lock();
        let mut stmt = conn
            .prepare("SELECT reasons, avoided_tokens FROM observations WHERE created_at >= ?1")?;
        let mut acc: std::collections::BTreeMap<String, u64> = std::collections::BTreeMap::new();
        let rows = stmt.query_map(params![since_unix], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? as u64))
        })?;
        for row in rows {
            let (json, avoided) = row?;
            let parsed: serde_json::Value =
                serde_json::from_str(&json).unwrap_or(serde_json::json!([]));
            if let Some(arr) = parsed.as_array() {
                if arr.is_empty() && avoided > 0 {
                    *acc.entry("unspecified".into()).or_default() += avoided;
                    continue;
                }
                for item in arr {
                    let label = item
                        .get("label")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unspecified");
                    let tokens = item.get("tokens").and_then(|v| v.as_u64()).unwrap_or(0);
                    *acc.entry(label.to_string()).or_default() += tokens;
                }
            }
        }
        Ok(acc.into_iter().collect())
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct TokenTotals {
    pub raw: u64,
    pub delivered: u64,
    pub avoided: u64,
}

impl TokenTotals {
    pub fn reduction_pct(self) -> u32 {
        if self.raw == 0 {
            return 0;
        }
        ((self.avoided as f64 / self.raw as f64) * 100.0).round() as u32
    }
}

fn map_observation(r: &rusqlite::Row<'_>) -> rusqlite::Result<Observation> {
    let reasons: String = r.get(11)?;
    Ok(Observation {
        id: r.get(0)?,
        session_id: r.get(1)?,
        event_type: r.get(2)?,
        tool_type: r.get(3)?,
        tool_name: r.get(4)?,
        uri: r.get(5)?,
        content_hash: r.get(6)?,
        raw_tokens: r.get(7)?,
        delivered_tokens: r.get(8)?,
        avoided_tokens: r.get(9)?,
        optimizer: r.get(10)?,
        reasons: serde_json::from_str(&reasons).unwrap_or(serde_json::json!([])),
        created_at: r.get(12)?,
        referenced: r.get::<_, i64>(13)? != 0,
        source_path: r.get(14)?,
        accessed_at: r.get::<_, i64>(15).unwrap_or(0),
    })
}

fn map_fingerprint(r: &rusqlite::Row<'_>) -> rusqlite::Result<FingerprintHit> {
    Ok(FingerprintHit {
        hash: r.get(0)?,
        uri: r.get(1)?,
        count: r.get(2)?,
        first_seen_at: r.get(3)?,
    })
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn first_signal_frame(frames: &[Frame]) -> Option<&str> {
    frames
        .iter()
        .find(|f| f.kind == "fail" || f.kind == "error")
        .map(|f| f.name.as_str())
}

fn clip_fts(body: &str) -> &str {
    const MAX: usize = 512_000;
    if body.len() <= MAX {
        return body;
    }
    let mut end = MAX;
    while end > 0 && !body.is_char_boundary(end) {
        end -= 1;
    }
    &body[..end]
}

fn best_frame(frames: &[Frame], needle: &str) -> Option<Frame> {
    let mut hits: Vec<&Frame> = frames
        .iter()
        .filter(|f| f.name.eq_ignore_ascii_case(needle))
        .collect();
    if hits.is_empty() {
        hits = frames
            .iter()
            .filter(|f| f.name.to_ascii_lowercase().contains(needle))
            .collect();
    }
    hits.into_iter()
        .max_by_key(|f| (f.end_line.saturating_sub(f.start_line), f.hint.len() as u32))
        .cloned()
}

/// Quote alphanumeric tokens so user input cannot break FTS5 MATCH syntax.
pub fn fts_query(raw: &str) -> Option<String> {
    let terms: Vec<String> = raw
        .split_whitespace()
        .map(|t| {
            t.chars()
                .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
                .collect::<String>()
        })
        .filter(|t| t.len() >= 2)
        .map(|t| format!("\"{t}\""))
        .collect();
    if terms.is_empty() {
        None
    } else {
        Some(terms.join(" OR "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn put_get_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(CtxPaths::from_root(dir.path().to_path_buf())).unwrap();
        let put = store.put_bytes(b"hello ctx").unwrap();
        assert_eq!(put.bytes, 9);
        let got = store.get_bytes(&put.hash).unwrap();
        assert_eq!(got, b"hello ctx");
        let again = store.put_bytes(b"hello ctx").unwrap();
        assert_eq!(again.hash, put.hash);
    }

    #[test]
    fn recent_pages_newest_first() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(CtxPaths::from_root(dir.path().to_path_buf())).unwrap();
        let a = store.put_bytes(b"alpha").unwrap();
        store
            .put_page(
                &ctx_protocol::CtxUri::new("shell", &a.hash),
                &a.hash,
                None,
                10,
            )
            .unwrap();
        let b = store.put_bytes(b"bravo").unwrap();
        store
            .put_page(
                &ctx_protocol::CtxUri::new("file", &b.hash),
                &b.hash,
                None,
                20,
            )
            .unwrap();
        let pages = store.recent_pages(8).unwrap();
        assert_eq!(pages.len(), 2);
        let kinds: Vec<_> = pages.iter().map(|p| p.kind.as_str()).collect();
        assert!(kinds.contains(&"file"), "{kinds:?}");
        assert!(kinds.contains(&"shell"), "{kinds:?}");
    }

    #[test]
    fn fts_query_quotes_and_drops_noise() {
        assert_eq!(
            fts_query("error 401").as_deref(),
            Some("\"error\" OR \"401\"")
        );
        assert_eq!(fts_query("a * AND").as_deref(), Some("\"AND\""));
        assert!(fts_query("x").is_none());
    }

    #[test]
    fn fts_index_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(CtxPaths::from_root(dir.path().to_path_buf())).unwrap();
        let put = store
            .put_bytes(b"auth login returned 401 unauthorized")
            .unwrap();
        let uri = ctx_protocol::CtxUri::new("shell", &put.hash);
        store.put_page(&uri, &put.hash, None, 12).unwrap();
        store
            .index_page(
                &uri.to_string(),
                "shell",
                "auth login returned 401 unauthorized",
            )
            .unwrap();
        let hits = store.search_fts("401", 8).unwrap();
        assert_eq!(hits.len(), 1, "{hits:?}");
        assert_eq!(hits[0].uri, uri.to_string());
        assert!(hits[0].snippet.contains("401"), "{}", hits[0].snippet);
    }

    #[test]
    fn frame_table_search_is_a_page_walk() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(CtxPaths::from_root(dir.path().to_path_buf())).unwrap();
        let uri = ctx_protocol::CtxUri::new("shell", "aaaaaaaaaaaa");
        store
            .replace_frames(
                &uri.page_key(),
                &[ctx_protocol::Frame::new("auth::login", "fail", 10, 18).with_hint("left: 401")],
            )
            .unwrap();
        let hits = store.search_frames("login", 8).unwrap();
        assert_eq!(hits.len(), 1, "{hits:?}");
        assert_eq!(hits[0].name, "auth::login");
        assert!(hits[0].hint.contains("401"));
    }

    #[test]
    fn fingerprint_near_dup_is_count_two() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(CtxPaths::from_root(dir.path().to_path_buf())).unwrap();
        let a = "left: 401\nright: 200\n";
        let b = "left: 401\n  right: 200\n";
        let ha = blake3_hex(a.as_bytes());
        let hb = blake3_hex(b.as_bytes());
        assert_ne!(ha, hb);
        let na = normalize_hash(a);
        let nb = normalize_hash(b);
        assert_eq!(na, nb);
        let first = store
            .remember_fingerprint(&ha, &na, Some("ctx://shell/a"))
            .unwrap();
        assert_eq!(first.count, 1);
        let second = store
            .remember_fingerprint(&hb, &nb, Some("ctx://shell/b"))
            .unwrap();
        assert!(second.count >= 2, "{second:?}");
        assert_eq!(second.uri.as_deref(), Some("ctx://shell/a"));
    }

    #[test]
    fn record_page_keeps_task_and_harness() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(CtxPaths::from_root(dir.path().to_path_buf())).unwrap();
        let put = store.put_bytes(b"auth login failed").unwrap();
        let uri = ctx_protocol::CtxUri::new("shell", &put.hash);
        store
            .record_page(RecordPage {
                uri: &uri,
                hash: &put.hash,
                body: "test auth::login ... FAILED\n",
                frames: &[ctx_protocol::Frame::new("auth::login", "fail", 1, 2)],
                raw_tokens: 12,
                harness: "claude-code",
                task: "auth login",
            })
            .unwrap();
        let pages = store.recent_pages(8).unwrap();
        assert_eq!(pages.len(), 1);
        assert_eq!(pages[0].task, "auth login");
        assert_eq!(pages[0].harness, "claude-code");
        assert_eq!(pages[0].summary.as_deref(), Some("auth::login"));
    }

    #[test]
    fn schema_v4_migrates_task_columns() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        std::fs::create_dir_all(&root).unwrap();
        let db = root.join("ctx.db");
        {
            let conn = rusqlite::Connection::open(&db).unwrap();
            conn.execute_batch(
                r#"
                CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
                CREATE TABLE pages (
                    uri TEXT PRIMARY KEY,
                    hash TEXT NOT NULL,
                    kind TEXT NOT NULL,
                    summary TEXT,
                    raw_tokens INTEGER NOT NULL,
                    created_at INTEGER NOT NULL
                );
                CREATE TABLE sessions (
                    id TEXT PRIMARY KEY,
                    harness TEXT NOT NULL,
                    started_at INTEGER NOT NULL,
                    ended_at INTEGER,
                    cwd TEXT,
                    metadata TEXT NOT NULL DEFAULT '{}'
                );
                INSERT INTO meta (key, value) VALUES ('schema_version', '4');
                INSERT INTO pages (uri, hash, kind, summary, raw_tokens, created_at)
                VALUES ('ctx://shell/old', 'h', 'shell', NULL, 10, 1);
                "#,
            )
            .unwrap();
        }
        let store = Store::open(CtxPaths::from_root(root)).unwrap();
        let pages = store.recent_pages(8).unwrap();
        assert_eq!(pages.len(), 1);
        assert_eq!(pages[0].task, "");
        assert_eq!(pages[0].harness, "");
        store.ensure_session("s1", "cursor", None).unwrap();
        store.set_session_task("s1", "auth login").unwrap();
        assert_eq!(store.session_task("s1").unwrap(), "auth login");
    }
}
