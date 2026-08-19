//! Deterministic reduction for git diff / log / status.

use crate::compact::is_diagnostic_line;

pub enum GitKind {
    Diff,
    Log,
    Status,
}

pub fn detect_git(text: &str, command: Option<&str>) -> Option<GitKind> {
    if let Some(cmd) = command {
        if looks_like_git_command(cmd, "diff") || looks_like_git_command(cmd, "show") {
            return Some(GitKind::Diff);
        }
        if looks_like_git_command(cmd, "log") {
            return Some(GitKind::Log);
        }
        if looks_like_git_command(cmd, "status") {
            return Some(GitKind::Status);
        }
    }
    if looks_like_diff(text) {
        return Some(GitKind::Diff);
    }
    if looks_like_log(text) {
        return Some(GitKind::Log);
    }
    if looks_like_status(text) {
        return Some(GitKind::Status);
    }
    None
}

fn looks_like_git_command(cmd: &str, sub: &str) -> bool {
    let c = cmd.to_ascii_lowercase();
    c.contains(&format!("git {sub}"))
        || c.contains(&format!("git --no-pager {sub}"))
        || c.contains(&format!("git.exe {sub}"))
}

fn looks_like_diff(text: &str) -> bool {
    text.lines()
        .filter(|l| l.starts_with("diff --git "))
        .count()
        >= 1
        && (text.contains("\n@@ ") || text.contains("\n+++ ") || text.contains("Binary files "))
}

fn looks_like_log(text: &str) -> bool {
    let mut commits = 0u32;
    for line in text.lines() {
        if is_commit_line(line) {
            commits += 1;
        }
    }
    commits >= 2 || (commits >= 1 && text.contains("\nAuthor:") && text.contains("\nDate:"))
}

fn looks_like_status(text: &str) -> bool {
    if looks_like_diff(text) {
        return false;
    }
    let t = text.trim_start();
    t.starts_with("On branch ")
        || text.contains("\nChanges not staged")
        || text.contains("\nChanges to be committed")
        || text.contains("\nUntracked files:")
        || text.contains("nothing to commit")
}

fn is_commit_line(line: &str) -> bool {
    let Some(rest) = line.strip_prefix("commit ") else {
        return false;
    };
    let hash = rest.split_whitespace().next().unwrap_or("");
    hash.len() >= 7 && hash.chars().all(|c| c.is_ascii_hexdigit())
}

pub fn reduce_diff(text: &str) -> String {
    let mut files: Vec<FileDiff> = Vec::new();
    let mut cur: Option<FileDiff> = None;
    let mut plus_total = 0u32;
    let mut minus_total = 0u32;

    for line in text.lines() {
        if let Some(path) = diff_path(line) {
            if let Some(prev) = cur.take() {
                plus_total += prev.plus;
                minus_total += prev.minus;
                files.push(prev);
            }
            cur = Some(FileDiff::new(path));
            continue;
        }
        let Some(file) = cur.as_mut() else {
            continue;
        };
        if line.starts_with("@@ ") {
            if file.hunks.len() < 6 {
                file.hunks.push(hunk_label(line));
            } else {
                file.extra_hunks += 1;
            }
            continue;
        }
        if line.starts_with("Binary files ") || line.contains("GIT binary patch") {
            file.binary = true;
            continue;
        }
        if line.starts_with("+++ ") || line.starts_with("--- ") || line.starts_with("index ") {
            continue;
        }
        if let Some(rest) = line.strip_prefix('+') {
            file.plus += 1;
            keep_change(file, '+', rest);
        } else if let Some(rest) = line.strip_prefix('-') {
            if line.starts_with("---") {
                continue;
            }
            file.minus += 1;
            keep_change(file, '-', rest);
        }
    }
    if let Some(prev) = cur.take() {
        plus_total += prev.plus;
        minus_total += prev.minus;
        files.push(prev);
    }

    if files.is_empty() {
        return crate::compact::diagnostic_excerpt(text, 24);
    }

    let mut body = format!("{} files  +{plus_total} -{minus_total}\n", files.len());
    for (i, f) in files.iter().enumerate() {
        if i >= 8 {
            body.push_str(&format!("… {} more files\n", files.len() - 8));
            break;
        }
        body.push_str(&format!("{}  +{} -{}\n", f.path, f.plus, f.minus));
        if f.binary {
            body.push_str("  binary\n");
            continue;
        }
        for h in &f.hunks {
            body.push_str("  ");
            body.push_str(h);
            body.push('\n');
        }
        if f.extra_hunks > 0 {
            body.push_str(&format!("  … {} hunks\n", f.extra_hunks));
        }
        for c in &f.changes {
            body.push_str("  ");
            body.push_str(c);
            body.push('\n');
        }
        if f.dropped > 0 {
            body.push_str(&format!("  … {} lines\n", f.dropped));
        }
    }
    body
}

struct FileDiff {
    path: String,
    plus: u32,
    minus: u32,
    hunks: Vec<String>,
    extra_hunks: u32,
    changes: Vec<String>,
    dropped: u32,
    binary: bool,
}

impl FileDiff {
    fn new(path: String) -> Self {
        Self {
            path,
            plus: 0,
            minus: 0,
            hunks: Vec::new(),
            extra_hunks: 0,
            changes: Vec::new(),
            dropped: 0,
            binary: false,
        }
    }
}

fn keep_change(file: &mut FileDiff, mark: char, rest: &str) {
    let t = rest.trim();
    if t.is_empty() {
        file.dropped += 1;
        return;
    }
    let noise = t.starts_with("use ") || t.starts_with("import ") || t.starts_with("//");
    let keep = is_diagnostic_line(t)
        || t.contains("TODO")
        || t.contains("FIXME")
        || (!noise && file.changes.len() < 8);
    if keep {
        let clip: String = t.chars().take(100).collect();
        file.changes.push(format!("{mark} {clip}"));
    } else {
        file.dropped += 1;
    }
}

fn diff_path(line: &str) -> Option<String> {
    let rest = line.strip_prefix("diff --git ")?;
    let mut parts = rest.split_whitespace();
    let _a = parts.next()?;
    let b = parts.next().unwrap_or(_a);
    let path = b.strip_prefix("b/").unwrap_or(b);
    if path.is_empty() {
        None
    } else {
        Some(path.to_string())
    }
}

fn hunk_label(line: &str) -> String {
    let t = line.trim();
    if let Some((_, ctx)) = t.split_once("@@") {
        let ctx = ctx.rsplit("@@").next().unwrap_or("").trim();
        if !ctx.is_empty() {
            return format!("@@ {ctx}");
        }
    }
    t.chars().take(60).collect()
}

pub fn reduce_log(text: &str) -> String {
    let mut commits: Vec<(String, String)> = Vec::new();
    let mut hash = String::new();
    let mut subject = String::new();
    for line in text.lines() {
        if is_commit_line(line) {
            if !hash.is_empty() {
                commits.push((std::mem::take(&mut hash), std::mem::take(&mut subject)));
            }
            hash = line
                .strip_prefix("commit ")
                .unwrap_or(line)
                .split_whitespace()
                .next()
                .unwrap_or("")
                .chars()
                .take(8)
                .collect();
            subject.clear();
            continue;
        }
        if line.starts_with("Author:") || line.starts_with("Date:") || line.starts_with("Merge:") {
            continue;
        }
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        if subject.is_empty() {
            subject = t.chars().take(72).collect();
        }
    }
    if !hash.is_empty() {
        commits.push((hash, subject));
    }
    if commits.is_empty() {
        return crate::compact::diagnostic_excerpt(text, 24);
    }
    let mut body = format!("{} commits\n", commits.len());
    for (h, s) in commits.iter().take(16) {
        if s.is_empty() {
            body.push_str(h);
            body.push('\n');
        } else {
            body.push_str(&format!("{h}  {s}\n"));
        }
    }
    if commits.len() > 16 {
        body.push_str(&format!("… {} more\n", commits.len() - 16));
    }
    body
}

pub fn reduce_status(text: &str) -> String {
    let mut branch = "";
    let mut ahead = "";
    let mut staged: Vec<&str> = Vec::new();
    let mut unstaged: Vec<&str> = Vec::new();
    let mut untracked: Vec<&str> = Vec::new();
    let mut section = "";
    for line in text.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("On branch ") {
            branch = rest;
            continue;
        }
        if t.starts_with("Your branch") {
            ahead = t;
            continue;
        }
        if t.starts_with("Changes to be committed") {
            section = "staged";
            continue;
        }
        if t.starts_with("Changes not staged") {
            section = "unstaged";
            continue;
        }
        if t.starts_with("Untracked files") {
            section = "untracked";
            continue;
        }
        if t.starts_with("no changes added")
            || t.starts_with("(")
            || t.starts_with("use \"git")
            || t.starts_with("nothing to commit")
        {
            continue;
        }
        if t.is_empty() {
            continue;
        }
        let file = t
            .strip_prefix("modified:")
            .or_else(|| t.strip_prefix("new file:"))
            .or_else(|| t.strip_prefix("deleted:"))
            .or_else(|| t.strip_prefix("renamed:"))
            .map(str::trim)
            .unwrap_or(t);
        match section {
            "staged" if staged.len() < 24 => staged.push(line.trim()),
            "unstaged" if unstaged.len() < 24 => unstaged.push(line.trim()),
            "untracked" if untracked.len() < 24 => untracked.push(file),
            _ => {}
        }
    }
    let mut body = String::new();
    if !branch.is_empty() {
        body.push_str("On branch ");
        body.push_str(branch);
        body.push('\n');
    }
    if !ahead.is_empty() {
        body.push_str(ahead);
        body.push('\n');
    }
    push_section(&mut body, "staged", &staged);
    push_section(&mut body, "unstaged", &unstaged);
    push_section(&mut body, "untracked", &untracked);
    if body.is_empty() {
        crate::compact::diagnostic_excerpt(text, 20)
    } else {
        body
    }
}

fn push_section(body: &mut String, name: &str, items: &[&str]) {
    if items.is_empty() {
        return;
    }
    body.push_str(&format!("{name}  {}\n", items.len()));
    for it in items.iter().take(16) {
        body.push_str("  ");
        body.push_str(it);
        body.push('\n');
    }
    if items.len() > 16 {
        body.push_str(&format!("  … {} more\n", items.len() - 16));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tokens::estimate_tokens;

    #[test]
    fn diff_keeps_hunk_and_assertion() {
        let raw = concat!(
            "diff --git a/src/auth.rs b/src/auth.rs\n",
            "index 111..222 100644\n",
            "--- a/src/auth.rs\n",
            "+++ b/src/auth.rs\n",
            "@@ -80,6 +80,8 @@ pub fn login() {\n",
            "     let x = 1;\n",
            "-    Ok(200)\n",
            "+    Err(401)\n",
            "+    // redirect_uri mismatch\n",
            " }\n",
            "diff --git a/src/lib.rs b/src/lib.rs\n",
            "--- a/src/lib.rs\n",
            "+++ b/src/lib.rs\n",
            "@@ -1,3 +1,4 @@\n",
            "+use std::io;\n",
        );
        let mut padded = raw.to_string();
        for i in 0..80 {
            padded.push_str(&format!("+ padding_line_{i}_aaaa\n"));
        }
        let out = reduce_diff(&padded);
        assert!(out.contains("src/auth.rs"), "{out}");
        assert!(out.contains("401") || out.contains("redirect_uri"), "{out}");
        assert!(out.contains("@@"), "{out}");
        assert!(!out.contains("padding_line_40"), "{out}");
        assert!(
            estimate_tokens(&out) < estimate_tokens(&padded) / 3,
            "{out}"
        );
    }

    #[test]
    fn log_is_subjects_not_bodies() {
        let mut raw = String::new();
        for i in 0..20 {
            raw.push_str(&format!(
                "commit {:040x}\nAuthor: A <a@b>\nDate: Mon Jan 1\n\n    fix oauth {}\n\n    long body line that should drop {}\n\n",
                i, i, "x".repeat(40)
            ));
        }
        let out = reduce_log(&raw);
        assert!(out.contains("20 commits"), "{out}");
        assert!(out.contains("fix oauth 0"), "{out}");
        assert!(!out.contains("long body line"), "{out}");
        assert!(!out.contains("Author:"), "{out}");
    }

    #[test]
    fn status_drops_help_text() {
        let raw = concat!(
            "On branch main\n",
            "Changes not staged for commit:\n",
            "  (use \"git add <file>...\" to update)\n",
            "        modified:   src/auth.rs\n",
            "        modified:   src/lib.rs\n",
            "Untracked files:\n",
            "  (use \"git add <file>...\" to include)\n",
            "        scratch.txt\n",
        );
        let out = reduce_status(raw);
        assert!(out.contains("main"), "{out}");
        assert!(out.contains("src/auth.rs"), "{out}");
        assert!(out.contains("scratch.txt"), "{out}");
        assert!(!out.contains("use \"git add"), "{out}");
    }

    #[test]
    fn detect_diff_beats_rustc_span_in_hunk() {
        let raw = concat!(
            "diff --git a/src/x.rs b/src/x.rs\n",
            "--- a/src/x.rs\n",
            "+++ b/src/x.rs\n",
            "@@ -1 +1 @@\n",
            "-error[E0308]: old\n",
            "+error[E0308]: new\n",
        );
        assert!(matches!(detect_git(raw, None), Some(GitKind::Diff)));
        assert!(matches!(
            detect_git(raw, Some("git diff HEAD")),
            Some(GitKind::Diff)
        ));
        let out = reduce_diff(raw);
        assert!(out.contains("src/x.rs"), "{out}");
        assert!(
            out.contains("E0308") || out.contains("error[E0308]"),
            "{out}"
        );
    }
}
