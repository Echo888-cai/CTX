//! Copy-on-write working set.
//!
//! Raw bytes stay intact (principle 1). Delivery can be a delta against the
//! previous generation of the same kind — like a CoW page, not a new blob.

use crate::tokens::estimate_tokens;

/// If `current` shares most lines with `previous`, return a compact delta.
pub fn cow_working_set(previous: &str, prev_uri: &str, current: &str) -> Option<String> {
    let prev_lines: Vec<&str> = previous.lines().collect();
    let curr_lines: Vec<&str> = current.lines().collect();
    if curr_lines.len() < 40 || prev_lines.len() < 40 {
        return None;
    }
    let prev_set: std::collections::HashSet<&str> = prev_lines.iter().copied().collect();
    let shared = curr_lines.iter().filter(|l| prev_set.contains(**l)).count();
    let ratio = shared as f64 / curr_lines.len() as f64;
    if ratio < 0.72 {
        return None;
    }

    let curr_set: std::collections::HashSet<&str> = curr_lines.iter().copied().collect();
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
        assert!(out.contains("FAILED"), "{out}");
        assert!(out.contains("401"), "{out}");
        assert!(!out.contains("test t12 ... ok"), "{out}");
        assert!(estimate_tokens(&out) < estimate_tokens(&curr) / 2);
    }

    #[test]
    fn unrelated_logs_are_not_cow() {
        let a = "alpha\n".repeat(50);
        let b = "beta\n".repeat(50);
        assert!(cow_working_set(&a, "ctx://shell/x", &b).is_none());
    }
}
