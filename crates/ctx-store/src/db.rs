use rusqlite::Connection;

use crate::Result;

const SCHEMA_VERSION: i64 = 6;

pub fn migrate(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS meta (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS pages (
            uri TEXT PRIMARY KEY,
            hash TEXT NOT NULL,
            kind TEXT NOT NULL,
            summary TEXT,
            raw_tokens INTEGER NOT NULL,
            created_at INTEGER NOT NULL,
            task TEXT NOT NULL DEFAULT '',
            harness TEXT NOT NULL DEFAULT ''
        );

        CREATE TABLE IF NOT EXISTS sessions (
            id TEXT PRIMARY KEY,
            harness TEXT NOT NULL,
            started_at INTEGER NOT NULL,
            ended_at INTEGER,
            cwd TEXT,
            metadata TEXT NOT NULL DEFAULT '{}',
            task TEXT NOT NULL DEFAULT '',
            remap INTEGER NOT NULL DEFAULT 0
        );

        CREATE TABLE IF NOT EXISTS blobs (
            hash TEXT PRIMARY KEY,
            bytes INTEGER NOT NULL,
            compressed_bytes INTEGER NOT NULL,
            created_at INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS observations (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            session_id TEXT NOT NULL,
            event_type TEXT NOT NULL,
            tool_type TEXT,
            tool_name TEXT,
            uri TEXT,
            content_hash TEXT NOT NULL,
            raw_tokens INTEGER NOT NULL,
            delivered_tokens INTEGER NOT NULL,
            avoided_tokens INTEGER NOT NULL,
            optimizer TEXT,
            reasons TEXT NOT NULL DEFAULT '[]',
            created_at INTEGER NOT NULL,
            referenced INTEGER NOT NULL DEFAULT 0,
            source_path TEXT,
            accessed_at INTEGER NOT NULL DEFAULT 0
        );

        CREATE TABLE IF NOT EXISTS fingerprints (
            hash TEXT PRIMARY KEY,
            normalized_hash TEXT,
            uri TEXT,
            first_seen_at INTEGER NOT NULL,
            last_seen_at INTEGER NOT NULL,
            count INTEGER NOT NULL DEFAULT 1
        );

        CREATE TABLE IF NOT EXISTS file_reads (
            path TEXT PRIMARY KEY,
            content_hash TEXT NOT NULL,
            last_uri TEXT,
            last_tokens INTEGER NOT NULL,
            regions TEXT,
            last_seen_at INTEGER NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_obs_session ON observations(session_id);
        CREATE INDEX IF NOT EXISTS idx_obs_created ON observations(created_at);
        CREATE INDEX IF NOT EXISTS idx_obs_uri ON observations(uri);
        CREATE INDEX IF NOT EXISTS idx_fp_norm ON fingerprints(normalized_hash);

        CREATE VIRTUAL TABLE IF NOT EXISTS pages_fts USING fts5(
            uri UNINDEXED,
            kind UNINDEXED,
            body,
            tokenize = 'unicode61'
        );

        CREATE TABLE IF NOT EXISTS frames (
            uri TEXT NOT NULL,
            name TEXT NOT NULL,
            kind TEXT NOT NULL,
            start_line INTEGER NOT NULL,
            end_line INTEGER NOT NULL,
            hint TEXT NOT NULL DEFAULT '',
            PRIMARY KEY (uri, name, start_line)
        );
        CREATE INDEX IF NOT EXISTS idx_frames_name ON frames(name);
        CREATE INDEX IF NOT EXISTS idx_frames_uri ON frames(uri);
        "#,
    )?;

    let current: i64 = conn
        .query_row(
            "SELECT value FROM meta WHERE key = 'schema_version'",
            [],
            |r| {
                let s: String = r.get(0)?;
                Ok(s.parse::<i64>().unwrap_or(0))
            },
        )
        .unwrap_or(0);

    if current < 2 {
        // Old DBs were created without these columns. Duplicate-column errors
        // are ignored so fresh installs (CREATE already has them) still migrate.
        let _ = conn.execute(
            "ALTER TABLE observations ADD COLUMN referenced INTEGER NOT NULL DEFAULT 0",
            [],
        );
        let _ = conn.execute("ALTER TABLE observations ADD COLUMN source_path TEXT", []);
    }

    if current < 3 {
        let _ = conn.execute(
            "ALTER TABLE observations ADD COLUMN accessed_at INTEGER NOT NULL DEFAULT 0",
            [],
        );
        let _ = conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_obs_uri ON observations(uri)",
            [],
        );
    }

    if current < 4 {
        let _ = conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS frames (
                uri TEXT NOT NULL,
                name TEXT NOT NULL,
                kind TEXT NOT NULL,
                start_line INTEGER NOT NULL,
                end_line INTEGER NOT NULL,
                hint TEXT NOT NULL DEFAULT '',
                PRIMARY KEY (uri, name, start_line)
            );
            CREATE INDEX IF NOT EXISTS idx_frames_name ON frames(name);
            CREATE INDEX IF NOT EXISTS idx_frames_uri ON frames(uri);
            "#,
        );
    }

    if current < 5 {
        let _ = conn.execute(
            "ALTER TABLE pages ADD COLUMN task TEXT NOT NULL DEFAULT ''",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE pages ADD COLUMN harness TEXT NOT NULL DEFAULT ''",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE sessions ADD COLUMN task TEXT NOT NULL DEFAULT ''",
            [],
        );
    }

    if current < 6 {
        let _ = conn.execute(
            "ALTER TABLE sessions ADD COLUMN remap INTEGER NOT NULL DEFAULT 0",
            [],
        );
    }

    if current < SCHEMA_VERSION {
        conn.execute(
            "INSERT INTO meta (key, value) VALUES ('schema_version', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [SCHEMA_VERSION.to_string()],
        )?;
    }
    Ok(())
}
