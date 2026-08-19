//! Copy-on-write working set.
//!
//! Raw bytes stay intact (principle 1). Delivery can be a delta against the
//! previous generation of the same kind — like a CoW page, not a new blob.

use std::collections::HashSet;

use std::hash::{Hash, Hasher};

use crate::compact::compact_block;
use crate::diff::diff_working_set;
use crate::frames::extract_frames;
use crate::symbols::{collect_symbol_spans, slice_span};
use crate::tokens::estimate_tokens;

/// If `current` shares structure with `previous`, return a compact delta.
pub fn cow_working_set(previous: &str, prev_uri: &str, current: &str) -> Option<String> {
    if let Some(text) = cow_frames(previous, prev_uri, current) {
        return Some(text);
    }
    if let Some(text) = cow_symbols(previous, prev_uri, current) {
        return Some(text);
    }
    if let Some(text) = diff_working_set(previous, current, 400) {
        return Some(format!("CoW vs {prev_uri}\n{text}"));
    }
    cow_lines(previous, prev_uri, current)
}

fn span_hash(body: &str) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    body.hash(&mut h);
    h.finish()
}

fn cow_symbols(previous: &str, prev_uri: &str, current: &str) -> Option<String> {
    let prev = collect_symbol_spans(previous);
    let curr = collect_symbol_spans(current);
    if prev.len() < 2 || curr.len() < 2 {
        return None;
    }
    let prev_map: std::collections::HashMap<&str, u64> = prev
        .iter()
        .map(|s| {
            (
                s.name.as_str(),
                span_hash(&slice_span(previous, s.start_line, s.end_line)),
            )
        })
        .collect();
    let curr_map: std::collections::HashMap<&str, (u64, u32, u32)> = curr
        .iter()
        .map(|s| {
            (
                s.name.as_str(),
                (
                    span_hash(&slice_span(current, s.start_line, s.end_line)),
                    s.start_line,
                    s.end_line,
                ),
            )
        })
        .collect();
    let mut added = Vec::new();
    let mut changed = Vec::new();
    for (name, (h, start, end)) in &curr_map {
        match prev_map.get(name) {
            None => added.push((*name, *start, *end)),
            Some(ph) if ph != h => changed.push((*name, *start, *end)),
            _ => {}
        }
    }
    let gone: Vec<&str> = prev_map
        .keys()
        .copied()
        .filter(|n| !curr_map.contains_key(n))
        .collect();
    if added.is_empty() && changed.is_empty() && gone.is_empty() {
        return None;
    }

    let mut out = format!("CoW vs {prev_uri}\n");
    for (name, start, end) in added.iter().take(8) {
        out.push_str(&format!("+ fn {name}  L{start}–{end}\n"));
    }
    for (name, start, end) in changed.iter().take(8) {
        out.push_str(&format!("~ fn {name}  L{start}–{end}\n"));
    }
    for name in gone.iter().take(8) {
        out.push_str(&format!("- fn {name}\n"));
    }

    let mut shown = 0u32;
    for (name, start, end) in changed.iter().chain(added.iter()).take(4) {
        let body = slice_span(current, *start, *end);
        let compact = if body.lines().count() > 24 {
            compact_block(&body, 20)
        } else {
            body
        };
        out.push('\n');
        out.push_str(&format!("#{name}\n"));
        out.push_str(&compact);
        out.push('\n');
        shown += 1;
        if shown >= 4 || estimate_tokens(&out) > 400 {
            break;
        }
    }
    let delivered = estimate_tokens(&out);
    let raw = estimate_tokens(current);
    if delivered + 80 >= raw {
        return None;
    }
    Some(out)
}

fn is_fail_kind(kind: &str) -> bool {
    matches!(kind, "fail" | "error")
}

fn cow_frames(previous: &str, prev_uri: &str, current: &str) -> Option<String> {
    let prev = extract_frames("shell", previous);
    let curr = extract_frames("shell", current);
    let prev_names: HashSet<&str> = prev
        .iter()
        .filter(|f| is_fail_kind(&f.kind))
        .map(|f| f.name.as_str())
        .collect();
    let curr_fail: Vec<_> = curr.iter().filter(|f| is_fail_kind(&f.kind)).collect();
    let curr_names: HashSet<&str> = curr_fail.iter().map(|f| f.name.as_str()).collect();
    if prev_names.is_empty() && curr_names.is_empty() {
        return None;
    }
    let added: Vec<&str> = curr_names.difference(&prev_names).copied().collect();
    let gone: Vec<&str> = prev_names.difference(&curr_names).copied().collect();
    if added.is_empty() && gone.is_empty() {
        return None;
    }

    let mut out = format!("CoW vs {prev_uri}\n");
    for name in added.iter().take(8) {
        out.push_str(&format!("+ FAIL {name}\n"));
    }
    for name in gone.iter().take(8) {
        out.push_str(&format!("- FAIL {name}\n"));
    }
    if added.len() > 8 {
        out.push_str(&format!("… {} new\n", added.len() - 8));
    }

    let lines: Vec<&str> = current.lines().collect();
    let mut shown = 0u32;
    for f in curr_fail
        .iter()
        .filter(|f| added.contains(&f.name.as_str()))
    {
        let start = f.start_line.saturating_sub(1) as usize;
        let end = (f.end_line as usize).clamp(start + 1, lines.len());
        if start >= lines.len() {
            continue;
        }
        let slice = lines[start..end].join("\n");
        let body = compact_block(&slice, 20);
        out.push('\n');
        out.push_str(&format!("#{}\n", f.name));
        out.push_str(&body);
        out.push('\n');
        shown += 1;
        if shown >= 4 || estimate_tokens(&out) > 400 {
            break;
        }
    }

    let delivered = estimate_tokens(&out);
    let raw = estimate_tokens(current);
    if delivered + 80 >= raw {
        return None;
    }
    Some(out)
}

fn cow_lines(previous: &str, prev_uri: &str, current: &str) -> Option<String> {
    let prev_lines: Vec<&str> = previous.lines().collect();
    let curr_lines: Vec<&str> = current.lines().collect();
    if curr_lines.len() < 40 || prev_lines.len() < 40 {
        return None;
    }
    let prev_set: HashSet<&str> = prev_lines.iter().copied().collect();
    let shared = curr_lines.iter().filter(|l| prev_set.contains(**l)).count();
    let ratio = shared as f64 / curr_lines.len() as f64;
    if ratio < 0.72 {
        return None;
    }

    let curr_set: HashSet<&str> = curr_lines.iter().copied().collect();
    let added: Vec<(usize, &str)> = curr_lines
        .iter()
        .enumerate()
        .filter(|(_, l)| !prev_set.contains(**l))
        .map(|(i, l)| (i, *l))
        .collect();
    if added.is_empty() || added.len() > curr_lines.len() / 2 {
        return None;
    }

    let mut out = format!(
        "CoW vs {prev_uri}  ({shared}/{} shared)\n",
        curr_lines.len()
    );
    let mut last: Option<usize> = None;
    for (i, line) in added.iter().take(80) {
        if let Some(prev) = last {
            if *i > prev + 1 {
                out.push_str("…\n");
            }
        }
        out.push_str(&format!("+ {:>5} | {line}\n", i + 1));
        last = Some(*i);
    }
    let removed: Vec<&str> = prev_lines
        .iter()
        .copied()
        .filter(|l| !curr_set.contains(l) && is_signal(l))
        .take(16)
        .collect();
    if !removed.is_empty() {
        out.push_str("gone:\n");
        for line in removed {
            out.push_str(&format!("- {line}\n"));
        }
    }

    let delivered = estimate_tokens(&out);
    let raw = estimate_tokens(current);
    if delivered + 80 >= raw {
        return None;
    }
    Some(out)
}

fn is_signal(line: &str) -> bool {
    let l = line.to_ascii_lowercase();
    l.contains("fail")
        || l.contains("error")
        || l.contains("panic")
        || l.contains("ok")
        || l.contains("pass")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delta_keeps_new_failure() {
        let mut prev = String::from("running 80 tests\n");
        let mut curr = String::from("running 80 tests\n");
        for i in 0..80 {
            prev.push_str(&format!("test t{i} ... ok\n"));
            if i == 7 {
                curr.push_str("test t7 ... FAILED\n");
            } else {
                curr.push_str(&format!("test t{i} ... ok\n"));
            }
        }
        prev.push_str("test result: ok. 80 passed; 0 failed\n");
        curr.push_str("---- t7 stdout ----\nleft: 401\n");
        curr.push_str("test result: FAILED. 79 passed; 1 failed\n");
        let out = cow_working_set(&prev, "ctx://shell/aaaa", &curr).expect("cow");
        assert!(out.contains("CoW vs"), "{out}");
        assert!(out.contains("FAILED") || out.contains("FAIL"), "{out}");
        assert!(out.contains("401"), "{out}");
        assert!(!out.contains("test t12 ... ok"), "{out}");
        assert!(estimate_tokens(&out) < estimate_tokens(&curr) / 2);
    }

    #[test]
    fn frame_cow_names_the_flipped_test() {
        let mut prev = String::from("running 40 tests\n");
        let mut curr = String::from("running 40 tests\n");
        for i in 0..40 {
            prev.push_str(&format!("test t{i} ... ok\n"));
            if i == 3 {
                curr.push_str("test auth::login ... FAILED\n");
            } else {
                curr.push_str(&format!("test t{i} ... ok\n"));
            }
        }
        prev.push_str("test result: ok. 40 passed; 0 failed\n");
        curr.push_str("---- auth::login stdout ----\nleft: 401\nright: 200\n");
        curr.push_str("test result: FAILED. 39 passed; 1 failed\n");
        let out = cow_working_set(&prev, "ctx://shell/prev", &curr).expect("cow");
        assert!(out.contains("+ FAIL auth::login"), "{out}");
        assert!(out.contains("401"), "{out}");
        assert!(!out.contains("test t12 ... ok"), "{out}");
    }

    #[test]
    fn unrelated_logs_are_not_cow() {
        let a = "alpha\n".repeat(50);
        let b = "beta\n".repeat(50);
        assert!(cow_working_set(&a, "ctx://shell/x", &b).is_none());
    }

    #[test]
    fn line_diff_cow_keeps_changed_line() {
        let mut prev = String::new();
        let mut curr = String::new();
        for i in 0..48 {
            prev.push_str(&format!("shared line {i} xxxxxxxxx\n"));
            if i == 24 {
                curr.push_str("shared line 24 CHANGED-TOKEN\n");
            } else {
                curr.push_str(&format!("shared line {i} xxxxxxxxx\n"));
            }
        }
        let out = cow_working_set(&prev, "ctx://shell/prev", &curr).expect("cow");
        assert!(out.contains("CoW vs"), "{out}");
        assert!(out.contains("CHANGED-TOKEN"), "{out}");
        assert!(out.contains("@@") || out.contains("+"), "{out}");
        assert!(!out.contains("shared line 3 xxxxxxxxx"), "{out}");
        assert!(estimate_tokens(&out) < estimate_tokens(&curr) / 2);
    }

    #[test]
    fn file_symbol_cow_names_changed_fn() {
        let mut prev = String::from("fn noise() { 1 }\nfn other() { 2 }\n");
        let mut curr = String::from("fn noise() { 1 }\nfn other() { 2 }\n");
        prev.push_str("pub fn login(x: i32) -> i32 {\n    200\n}\n");
        curr.push_str("pub fn login(x: i32) -> i32 {\n    401\n}\n");
        for i in 0..24 {
            prev.push_str(&format!("fn extra_{i}() {{ {i} }}\n"));
            curr.push_str(&format!("fn extra_{i}() {{ {i} }}\n"));
        }
        let out = cow_working_set(&prev, "ctx://file/prev", &curr).expect("cow");
        assert!(out.contains("~ fn login"), "{out}");
        assert!(out.contains("401"), "{out}");
        assert!(!out.contains("fn extra_12"), "{out}");
        assert!(estimate_tokens(&out) < estimate_tokens(&curr) / 2);
    }
}
