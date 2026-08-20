//! Workspace base snapshot. File bodies stay in CAS; the snapshot is a path map.

use std::fs;
use std::path::Path;
use std::process::Command;

use ctx_store::blake3_hex;
use serde_json::json;

pub fn capture(cwd: Option<&Path>) -> (String, i64, String) {
    let Some(root) = cwd.filter(|p| p.is_dir()) else {
        return ("none".into(), 0, "{}".into());
    };
    let files = list_files(Some(root));
    let mut manifest = serde_json::Map::new();
    for (path, sig) in files.iter().take(2000) {
        manifest.insert(path.clone(), json!(sig));
    }
    let body = serde_json::Value::Object(manifest).to_string();
    let id = blake3_hex(body.as_bytes());
    let short = if id.len() >= 12 { id[..12].to_string() } else { id };
    (short, files.len() as i64, body)
}

fn list_files(cwd: Option<&Path>) -> Vec<(String, String)> {
    if let Some(git) = git_files(cwd) {
        return git;
    }
    walk(cwd.unwrap_or(Path::new(".")), 400)
}

fn git_files(cwd: Option<&Path>) -> Option<Vec<(String, String)>> {
    let mut cmd = Command::new("git");
    cmd.args(["ls-files", "-z"]);
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    let out = cmd.output().ok()?;
    if !out.status.success() {
        return None;
    }
    let mut files = Vec::new();
    for path in out.stdout.split(|b| *b == 0) {
        if path.is_empty() {
            continue;
        }
        let rel = String::from_utf8_lossy(path).into_owned();
        let full = match cwd {
            Some(dir) => dir.join(&rel),
            None => Path::new(&rel).to_path_buf(),
        };
        files.push((rel, file_sig(&full)));
        if files.len() >= 2000 {
            break;
        }
    }
    Some(files)
}

fn walk(root: &Path, cap: usize) -> Vec<(String, String)> {
    let mut out = Vec::new();
    walk_inner(root, root, cap, &mut out);
    out
}

fn walk_inner(root: &Path, dir: &Path, cap: usize, out: &mut Vec<(String, String)>) {
    if out.len() >= cap {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        if out.len() >= cap {
            break;
        }
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') || name == "target" || name == "node_modules" {
            continue;
        }
        if path.is_dir() {
            walk_inner(root, &path, cap, out);
        } else if let Ok(rel) = path.strip_prefix(root) {
            out.push((rel.display().to_string(), file_sig(&path)));
        }
    }
}

fn file_sig(path: &Path) -> String {
    let Ok(meta) = fs::metadata(path) else {
        return "missing".into();
    };
    if meta.len() <= 256 * 1024 {
        if let Ok(bytes) = fs::read(path) {
            return blake3_hex(&bytes);
        }
    }
    format!("{}:{}", meta.len(), mtime(&meta))
}

fn mtime(meta: &fs::Metadata) -> u64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_is_stable_for_same_tree() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.rs"), "fn a() {}").unwrap();
        let (id1, n1, _) = capture(Some(dir.path()));
        let (id2, n2, _) = capture(Some(dir.path()));
        assert_eq!(id1, id2);
        assert_eq!(n1, n2);
        assert!(n1 >= 1);
        std::fs::write(dir.path().join("a.rs"), "fn a() { 2 }").unwrap();
        let (id3, _, _) = capture(Some(dir.path()));
        assert_ne!(id1, id3);
    }

    #[test]
    fn missing_cwd_is_a_named_empty_snapshot() {
        let (id, n, body) = capture(None);
        assert_eq!(id, "none");
        assert_eq!(n, 0);
        assert_eq!(body, "{}");
    }
}
