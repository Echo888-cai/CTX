use std::path::{Path, PathBuf};

use crate::{Result, StoreError};

pub const DEFAULT_HOME_ENV: &str = "CTX_HOME";

#[derive(Debug, Clone)]
pub struct CtxPaths {
    root: PathBuf,
    /// Legacy flag. `default_home` never sets this; an unwritable home is an error.
    pub fallback: bool,
}

impl CtxPaths {
    pub fn from_root(root: PathBuf) -> Self {
        Self {
            root,
            fallback: false,
        }
    }

    pub fn default_home() -> Result<Self> {
        let preferred = if let Ok(p) = std::env::var(DEFAULT_HOME_ENV) {
            Self::from_root(PathBuf::from(p))
        } else {
            let home = dirs::home_dir().ok_or(StoreError::NoHome)?;
            Self::from_root(home.join(".ctx"))
        };
        Self::open_preferred(preferred)
    }

    /// Accept this root if it is writable. Never fall back to another tree.
    pub fn open_preferred(preferred: Self) -> Result<Self> {
        if can_write(&preferred) {
            return Ok(preferred);
        }
        Err(StoreError::HomeNotWritable {
            path: preferred.root.display().to_string(),
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn store_dir(&self) -> PathBuf {
        self.root.join("store")
    }

    pub fn db_path(&self) -> PathBuf {
        self.root.join("ctx.db")
    }

    pub fn config_path(&self) -> PathBuf {
        self.root.join("config.json")
    }

    pub fn prices_path(&self) -> PathBuf {
        self.root.join("prices.json")
    }

    pub fn official_prices_path(&self) -> PathBuf {
        self.root.join("prices.official.json")
    }

    pub fn snapshots_dir(&self) -> PathBuf {
        self.root.join("snapshots")
    }

    pub fn versions_dir(&self) -> PathBuf {
        self.root.join("versions")
    }

    pub fn ensure(&self) -> Result<()> {
        std::fs::create_dir_all(self.store_dir())?;
        std::fs::create_dir_all(self.snapshots_dir())?;
        Ok(())
    }
}

/// Probe: can we create the tree and write a byte? Cursor sandboxes often block `~/.ctx`.
pub fn can_write(paths: &CtxPaths) -> bool {
    if paths.ensure().is_err() {
        return false;
    }
    let probe = paths.root().join(".write-ok");
    match std::fs::write(&probe, b"ok") {
        Ok(()) => {
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_root_is_not_fallback() {
        let p = CtxPaths::from_root(PathBuf::from("/tmp/ctx-test"));
        assert!(!p.fallback);
        assert_eq!(p.db_path(), PathBuf::from("/tmp/ctx-test/ctx.db"));
    }

    #[test]
    fn unwritable_preferred_is_detected() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("not-a-dir");
        std::fs::write(&file, b"x").unwrap();
        let paths = CtxPaths::from_root(file);
        assert!(!can_write(&paths));
    }

    #[test]
    fn unwritable_preferred_does_not_fallback() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("not-a-dir");
        std::fs::write(&file, b"x").unwrap();
        let err = CtxPaths::open_preferred(CtxPaths::from_root(file.clone())).unwrap_err();
        match err {
            crate::StoreError::HomeNotWritable { path } => {
                assert!(path.contains("not-a-dir"), "{path}");
            }
            other => panic!("expected HomeNotWritable, got {other}"),
        }
        assert!(!tmp.path().join(".ctx").exists());
    }
}
