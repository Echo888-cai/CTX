//! Quality-preserving compaction: keep diagnostics, drop envelope noise.
//!
//! The model needs file:line, expected/actual, panic messages, and failed
//! names. It does not need backtraces, marketing copy, or duplicated indexes.

/// Lines that usually decide the next edit.
pub fn is_diagnostic_line(line: &str) -> bool {
    let t = line.trim();
    if t.is_empty() {
        return false;
    }
    if t.starts_with("--> ")
        || t.starts_with("error[")
        || t.starts_with("error: ")
        || t.starts_with("error TS")
        || t.contains("error TS")
        || t.starts_with("FAILED ")
        || t.starts_with("FAIL ")
        || t.starts_with("--- FAIL:")
        || t.starts_with("FAIL\t")
        || t.starts_with("thread '")
        || t.contains("panicked at")
        || t.starts_with("assertion ")
        || t.starts_with("assert ")
        || t.starts_with("E   ")
        || t.contains("^^^^^")
        || t.contains("^^^^^^")
    {
        return true;
    }
    let l = t.to_ascii_lowercase();
    if l.contains("left:") || l.contains("right:") {
        return true;
    }
    if l.starts_with("expected:") || l.starts_with("received:") {
        return true;
    }
    if l.contains("expected `") || l.contains("expected:") && l.contains("found") {
        return true;
    }
    if l.contains("assertionerror") || l.contains("assert ") && l.contains("==") {
        return true;
    }
    if l.contains("mismatched types") || l.contains("cannot find") {
        return true;
    }
    false
}

pub fn is_rust_frame_line(line: &str) -> bool {
    let t = line.trim();
    let mut parts = t.splitn(2, ':');
    let n = parts.next().unwrap_or("");
    let rest = parts.next().unwrap_or("");
    !n.is_empty()
        && n.chars().all(|c| c.is_ascii_digit())
        && (rest.contains("::") || rest.contains("rust_") || rest.contains("core::"))
}

/// Drop rustc/libtest stack dumps. Keep `panicked at` + the assertion.
pub fn strip_backtraces(text: &str) -> String {
    let mut out = Vec::new();
    let mut in_bt = false;
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with("stack backtrace:") {
            in_bt = true;
            continue;
        }
        if t.starts_with("note: run with `RUST_BACKTRACE")
            || t.starts_with("note: Some details are omitted")
        {
            continue;
        }
        if in_bt {
            if is_rust_frame_line(t) || t.starts_with("at ") || t.is_empty() {
                continue;
            }
            in_bt = false;
        }
        out.push(line);
    }
    while out.last().is_some_and(|l| l.trim().is_empty()) {
        out.pop();
    }
    out.join("\n")
}

/// Keep a diagnostic block under `budget` lines without dropping the assert.
pub fn compact_block(text: &str, budget: usize) -> String {
    let stripped = strip_backtraces(text);
    let budget = budget.max(6);
    let lines: Vec<&str> = stripped.lines().collect();
    if lines.len() <= budget {
        return stripped.trim_end().to_string();
    }

    let n = lines.len();
    let mut keep = std::collections::BTreeSet::new();
    for i in 0..n.min(3) {
        keep.insert(i);
    }
    for i in n.saturating_sub(2)..n {
        keep.insert(i);
    }
    for (i, line) in lines.iter().enumerate() {
        if is_diagnostic_line(line) {
            keep.insert(i);
            if i > 0 {
                keep.insert(i - 1);
            }
            if i + 1 < n {
                keep.insert(i + 1);
            }
        }
    }

    if keep.len() > budget {
        let mut ranked: Vec<usize> = keep.iter().copied().collect();
        ranked.sort_by_key(|i| {
            let diagnostic = is_diagnostic_line(lines[*i]);
            let edge = *i < 3 || *i + 2 >= n;
            (!diagnostic, !edge, *i)
        });
        ranked.truncate(budget);
        keep = ranked.into_iter().collect();
    }

    let mut out = String::new();
    let mut last: Option<usize> = None;
    for i in keep {
        if let Some(prev) = last {
            if i > prev + 1 {
                out.push_str("…\n");
            }
        }
        out.push_str(lines[i]);
        out.push('\n');
        last = Some(i);
    }
    out.trim_end().to_string()
}

/// Path token from a mapped-file line (`uri  path` or `path  (ctx_read)`).
pub fn map_path_token(mapped: &str) -> &str {
    mapped
        .split_whitespace()
        .find(|p| {
            let l = p.to_ascii_lowercase();
            let looks_path = p.contains('/') || p.contains('\\') || has_source_ext(&l);
            looks_path && !p.starts_with("ctx://") && !p.starts_with('(')
        })
        .unwrap_or_else(|| mapped.split_whitespace().next().unwrap_or(mapped.trim()))
}

fn has_source_ext(lower: &str) -> bool {
    lower.ends_with(".rs")
        || lower.ends_with(".py")
        || lower.ends_with(".ts")
        || lower.ends_with(".js")
        || lower.ends_with(".go")
        || lower.ends_with(".tsx")
}

fn select_indices(
    lines: &[&str],
    head: usize,
    tail: usize,
    diag_cap: u32,
) -> std::collections::BTreeSet<usize> {
    let n = lines.len();
    let mut keep = std::collections::BTreeSet::new();
    for i in 0..n.min(head) {
        keep.insert(i);
    }
    for i in n.saturating_sub(tail)..n {
        keep.insert(i);
    }
    let mut diag = 0u32;
    for (i, line) in lines.iter().enumerate() {
        if is_diagnostic_line(line) {
            keep.insert(i);
            diag += 1;
            if diag >= diag_cap {
                break;
            }
        }
    }
    keep
}

fn join_kept(lines: &[&str], keep: &std::collections::BTreeSet<usize>) -> String {
    let mut out = String::new();
    let mut last: Option<usize> = None;
    for i in keep {
        if let Some(prev) = last {
            if *i > prev + 1 {
                out.push_str("…\n");
            }
        }
        out.push_str(lines[*i]);
        out.push('\n');
        last = Some(*i);
    }
    out
}

/// Head/tail + diagnostic lines. Identity when the page is already small.
pub fn diagnostic_preview(text: &str, identity_below: usize, diag_cap: u32) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let n = lines.len();
    if n <= identity_below {
        return text.to_string();
    }
    let keep = select_indices(&lines, 12, 16, diag_cap);
    format!("{}/{} lines\n\n{}", keep.len(), n, join_kept(&lines, &keep))
}

/// Same selection, header only when lines were dropped (shell generic fallback).
pub fn diagnostic_excerpt(text: &str, diag_cap: u32) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let n = lines.len();
    let keep = select_indices(&lines, 12, 16, diag_cap);
    let mut body = String::new();
    if keep.len() < n {
        body.push_str(&format!("{}/{} lines\n\n", keep.len(), n));
    }
    body.push_str(&join_kept(&lines, &keep));
    body
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_assert_drops_backtrace() {
        let raw = "\
thread 'auth::login' panicked at src/auth.rs:82:5:
assertion `left == right` failed
  left: 401
  right: 200
stack backtrace:
   0: rust_begin_unwind
   1: core::panicking::panic_fmt
   2: app::auth::login
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
";
        let out = compact_block(raw, 16);
        assert!(out.contains("401"), "{out}");
        assert!(out.contains("src/auth.rs:82"), "{out}");
        assert!(!out.contains("rust_begin_unwind"), "{out}");
        assert!(!out.contains("RUST_BACKTRACE"), "{out}");
    }

    #[test]
    fn short_block_is_identity() {
        let raw = "left: 401\nright: 200";
        assert_eq!(compact_block(raw, 16), raw);
    }
}
