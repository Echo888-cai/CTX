//! Line-level Myers diff for Copy-on-Write working sets.
//!
//! Raw bytes stay intact. Delivery is a compact hunk list with 2 lines of
//! context — like `git diff`, cheap enough to run on every ingest.

use crate::tokens::estimate_tokens;

const CONTEXT: usize = 2;
const MAX_D: i32 = 256;
const MAX_LINES: usize = 8_000;

/// Compact line delta. `None` when the diff is not cheaper than `curr`,
/// when the files barely overlap, or when only trivia (whitespace/comments) changed.
pub fn diff_working_set(prev: &str, curr: &str, max_tokens: u32) -> Option<String> {
    let a: Vec<&str> = prev.lines().collect();
    let b: Vec<&str> = curr.lines().collect();
    if a.len() < 8 || b.len() < 8 {
        return None;
    }
    if a.len() + b.len() > MAX_LINES * 2 {
        return None;
    }
    let prev_set: std::collections::HashSet<&str> = a.iter().copied().collect();
    let shared = b.iter().filter(|l| prev_set.contains(**l)).count();
    let ratio = shared as f64 / b.len() as f64;
    if ratio < 0.72 {
        return None;
    }

    let edits = myers(&a, &b)?;
    let hunks = collect_hunks(&a, &b, &edits);
    if hunks.is_empty() {
        return None;
    }
    if hunks.iter().all(hunk_is_trivia) {
        return None;
    }

    let mut out = String::from("line diff\n");
    let mut used = estimate_tokens(&out);
    let budget = max_tokens.max(80);
    let mut shown = 0u32;
    for h in &hunks {
        if h.trivia_only {
            continue;
        }
        let block = render_hunk(h);
        let cost = estimate_tokens(&block).saturating_add(1);
        if used.saturating_add(cost) > budget && shown > 0 {
            out.push_str(&format!("… {} more hunks\n", hunks.len() as u32 - shown));
            break;
        }
        out.push_str(&block);
        used = used.saturating_add(cost);
        shown += 1;
        if shown >= 24 {
            break;
        }
    }
    let delivered = estimate_tokens(&out);
    let raw = estimate_tokens(curr);
    if delivered + 80 >= raw {
        return None;
    }
    Some(out)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Edit {
    Keep { a: usize, b: usize },
    Del { a: usize },
    Ins { b: usize },
}

struct Hunk {
    a_start: usize,
    b_start: usize,
    lines: Vec<HunkLine>,
    trivia_only: bool,
}

enum HunkLine {
    Ctx(String),
    Del(String),
    Ins(String),
}

impl HunkLine {
    fn is_change(&self) -> bool {
        !matches!(self, HunkLine::Ctx(_))
    }
}

fn hunk_is_trivia(h: &Hunk) -> bool {
    h.trivia_only
}

fn is_trivia(line: &str) -> bool {
    let t = line.trim();
    t.is_empty()
        || t.starts_with("//")
        || t.starts_with('#')
        || t.starts_with("/*")
        || t.starts_with('*')
        || t.starts_with("<!--")
}

fn myers(a: &[&str], b: &[&str]) -> Option<Vec<Edit>> {
    let n = a.len() as i32;
    let m = b.len() as i32;
    let max = (n + m).min(MAX_D);
    if max < 0 {
        return Some(Vec::new());
    }
    let offset = max as usize;
    let size = 2 * offset + 1;
    let mut v = vec![0i32; size];
    let mut trace: Vec<Vec<i32>> = Vec::new();

    for d in 0..=max {
        trace.push(v.clone());
        let mut k = -d;
        while k <= d {
            let idx = (k + max) as usize;
            let mut x = if k == -d || (k != d && v[idx - 1] < v[idx + 1]) {
                v[idx + 1]
            } else {
                v[idx - 1] + 1
            };
            let mut y = x - k;
            while x < n && y < m && a[x as usize] == b[y as usize] {
                x += 1;
                y += 1;
            }
            v[idx] = x;
            if x >= n && y >= m {
                return Some(backtrack(&trace, a, b, d));
            }
            k += 2;
        }
    }
    None
}

fn backtrack(trace: &[Vec<i32>], a: &[&str], b: &[&str], d_end: i32) -> Vec<Edit> {
    let n = a.len() as i32;
    let m = b.len() as i32;
    let mut x = n;
    let mut y = m;
    let mut edits = Vec::new();
    let max = (n + m).min(MAX_D);

    for d in (0..=d_end).rev() {
        let v = &trace[d as usize];
        let k = x - y;
        let idx = (k + max) as usize;
        let prev_k = if k == -d
            || (k != d && d > 0 && v[idx - 1] < v.get(idx + 1).copied().unwrap_or(i32::MIN))
        {
            k + 1
        } else {
            k - 1
        };
        let prev_x = if d == 0 {
            0
        } else {
            let pidx = (prev_k + max) as usize;
            v.get(pidx).copied().unwrap_or(0)
        };
        let prev_y = prev_x - prev_k;
        while x > prev_x && y > prev_y {
            x -= 1;
            y -= 1;
            edits.push(Edit::Keep {
                a: x as usize,
                b: y as usize,
            });
        }
        if d == 0 {
            break;
        }
        if x == prev_x {
            y -= 1;
            edits.push(Edit::Ins { b: y as usize });
        } else {
            x -= 1;
            edits.push(Edit::Del { a: x as usize });
        }
    }
    edits.reverse();
    edits
}

fn collect_hunks(a: &[&str], b: &[&str], edits: &[Edit]) -> Vec<Hunk> {
    let mut changed: Vec<bool> = vec![false; edits.len()];
    for (i, e) in edits.iter().enumerate() {
        changed[i] = !matches!(e, Edit::Keep { .. });
    }
    let mut hunks = Vec::new();
    let mut i = 0usize;
    while i < edits.len() {
        if !changed[i] {
            i += 1;
            continue;
        }
        let start = i.saturating_sub(CONTEXT);
        let mut end = i + 1;
        while end < edits.len() {
            let next_change = (end..edits.len()).find(|&j| changed[j]);
            match next_change {
                Some(j) if j <= end + CONTEXT * 2 => end = j + 1,
                _ => break,
            }
        }
        end = (end + CONTEXT).min(edits.len());
        let slice = &edits[start..end];
        let mut lines = Vec::new();
        let mut trivia_only = true;
        let mut a_start = a.len();
        let mut b_start = b.len();
        for e in slice {
            match *e {
                Edit::Keep { a: ai, b: bi } => {
                    a_start = a_start.min(ai);
                    b_start = b_start.min(bi);
                    lines.push(HunkLine::Ctx(a[ai].to_string()));
                }
                Edit::Del { a: ai } => {
                    a_start = a_start.min(ai);
                    if !is_trivia(a[ai]) {
                        trivia_only = false;
                    }
                    lines.push(HunkLine::Del(a[ai].to_string()));
                }
                Edit::Ins { b: bi } => {
                    b_start = b_start.min(bi);
                    if !is_trivia(b[bi]) {
                        trivia_only = false;
                    }
                    lines.push(HunkLine::Ins(b[bi].to_string()));
                }
            }
        }
        if lines.iter().any(HunkLine::is_change) {
            hunks.push(Hunk {
                a_start: a_start.saturating_add(1),
                b_start: b_start.saturating_add(1),
                lines,
                trivia_only,
            });
        }
        i = end;
    }
    hunks
}

fn render_hunk(h: &Hunk) -> String {
    let mut del = 0u32;
    let mut ins = 0u32;
    for l in &h.lines {
        match l {
            HunkLine::Del(_) => del += 1,
            HunkLine::Ins(_) => ins += 1,
            HunkLine::Ctx(_) => {}
        }
    }
    let mut out = format!(
        "@@ -{},{} +{},{} @@\n",
        h.a_start,
        del + h
            .lines
            .iter()
            .filter(|l| matches!(l, HunkLine::Ctx(_)))
            .count() as u32,
        h.b_start,
        ins + h
            .lines
            .iter()
            .filter(|l| matches!(l, HunkLine::Ctx(_)))
            .count() as u32
    );
    for l in &h.lines {
        match l {
            HunkLine::Ctx(s) => {
                out.push(' ');
                out.push_str(s);
                out.push('\n');
            }
            HunkLine::Del(s) => {
                out.push('-');
                out.push_str(s);
                out.push('\n');
            }
            HunkLine::Ins(s) => {
                out.push('+');
                out.push_str(s);
                out.push('\n');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hunk_keeps_changed_line_and_context() {
        let mut prev = String::new();
        let mut curr = String::new();
        for i in 0..40 {
            prev.push_str(&format!("line {i} shared\n"));
            if i == 20 {
                curr.push_str("line 20 CHANGED\n");
            } else {
                curr.push_str(&format!("line {i} shared\n"));
            }
        }
        let out = diff_working_set(&prev, &curr, 400).expect("diff");
        assert!(out.contains("CHANGED"), "{out}");
        assert!(out.contains("@@"), "{out}");
        assert!(out.contains("line 19 shared"), "{out}");
        assert!(!out.contains("line 0 shared"), "{out}");
        assert!(estimate_tokens(&out) < estimate_tokens(&curr) / 2);
    }

    #[test]
    fn trivia_only_is_none() {
        let mut prev = String::new();
        let mut curr = String::new();
        for i in 0..40 {
            prev.push_str(&format!("line {i} shared\n"));
            curr.push_str(&format!("line {i} shared\n"));
        }
        prev.push_str("// old comment\n");
        curr.push_str("// new comment\n");
        for i in 40..50 {
            prev.push_str(&format!("line {i} shared\n"));
            curr.push_str(&format!("line {i} shared\n"));
        }
        assert!(diff_working_set(&prev, &curr, 400).is_none());
    }

    #[test]
    fn unrelated_is_none() {
        let a = "alpha\n".repeat(50);
        let b = "beta\n".repeat(50);
        assert!(diff_working_set(&a, &b, 400).is_none());
    }
}
