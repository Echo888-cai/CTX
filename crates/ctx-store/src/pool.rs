//! Read-side SQLite pool. Dashboard, MCP, and search take connections here;
//! ingest keeps the exclusive write connection.

use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::Connection;

#[derive(Debug, Clone)]
pub struct SqliteConnectionManager {
    path: PathBuf,
    query_only: bool,
}

impl SqliteConnectionManager {
    pub fn query_only(path: impl AsRef<Path>) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
            query_only: true,
        }
    }
}

impl r2d2::ManageConnection for SqliteConnectionManager {
    type Connection = Connection;
    type Error = rusqlite::Error;

    fn connect(&self) -> Result<Connection, rusqlite::Error> {
        let conn = Connection::open(&self.path)?;
        conn.busy_timeout(Duration::from_millis(5_000))?;
        if self.query_only {
            let _ = conn.pragma_update(None, "query_only", 1);
        }
        Ok(conn)
    }

    fn is_valid(&self, conn: &mut Connection) -> Result<(), rusqlite::Error> {
        conn.query_row("SELECT 1", [], |_| Ok(()))?;
        Ok(())
    }

    fn has_broken(&self, _conn: &mut Connection) -> bool {
        false
    }
}
