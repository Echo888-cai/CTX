//! Page-fault retrieval: bring a region, not the whole disk.

use ctx_optimizer::{compact_block, estimate_tokens};

/// Below this, `ctx_fetch` without a query returns the full page.
pub const FULL_PAGE_TOKENS: u32 = 1_200;

/// Select lines around query terms. This is the v0.1 page fault.
pub fn page_in(text: &str, query: &str) -> String {
    let q = query.to_ascii_lowercase();
    let terms: Vec<&str> = q.split_whitespace().filter(|t| !t.is_empty()).collect();
    if terms.is_empty() {
        return bounded_preview(text, "ctx://");
    }
    let lines: Vec<&str> = text.lines().collect();
    let mut hits = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let l = line.to_ascii_lowercase();
        if terms.iter().any(|t| l.contains(t)) {
            hits.push(i);
        }
    }
    if hits.is_empty() {
        return format!("no match for {query:?}. query=* for the full page.\n");
    }
    let mut pad = match hits.len() {
        1 => 5usize,
        2..=4 => 3,
        _ => 2,
    };
    let mut keep = collect_windows(&hits, lines.len(), pad);
    while keep.len() > 72 && pad > 1 {
        pad -= 1;
        keep = collect_windows(&hits, lines.len(), pad);
    }
    let mut out = format!("{query:?}  {} hits\n\n", hits.len());
    let mut last: Option<usize> = None;
    for i in keep {
        if let Some(prev) = last {
            if i > prev + 1 {
                out.push_str("…\n");
            }
        }
        out.push_str(&format!("{:>6} | {}\n", i + 1, lines[i]));
        last = Some(i);
    }
    out
}

fn collect_windows(hits: &[usize], n: usize, pad: usize) -> std::collections::BTreeSet<usize> {
    let mut keep = std::collections::BTreeSet::new();
    for i in hits {
        let start = i.saturating_sub(pad);
        let end = (*i + pad + 1).min(n);
        for j in start..end {
            keep.insert(j);
        }
    }
    keep
}

/// Slice a named frame (page-table walk).
pub fn frame_slice(text: &str, uri: &str, name: &str, start_line: u32, end_line: u32) -> String {
    let lines: Vec<&str> = text.lines().collect();
    if lines.is_empty() {
        return format!("empty {uri}#{name}\n");
    }
    let start = (start_line.saturating_sub(1) as usize).min(lines.len().saturating_sub(1));
    let end = (end_line as usize).clamp(start + 1, lines.len());
    let slice = lines[start..end].join("\n");
    let body = compact_block(&slice, 28);
    format!("{uri}#{name}  L{}–{}\n\n{body}\n", start + 1, end)
}

/// Default fetch: small pages in full; large pages as a working-set preview.
pub fn bounded_preview(text: &str, uri: &str) -> String {
    bounded_preview_frames(text, uri, &[])
}

pub fn bounded_preview_frames(
    text: &str,
    uri: &str,
    frames: &[(String, String, u32, u32)],
) -> String {
    let tokens = estimate_tokens(text);
    if tokens <= FULL_PAGE_TOKENS && frames.is_empty() {
        return text.to_string();
    }
    let mut out = format!("{uri}  {tokens} stored\n");
    if !frames.is_empty() {
        let names: Vec<String> = frames
            .iter()
            .take(12)
            .map(|(n, _, _, _)| format!("#{n}"))
            .collect();
        out.push_str(&names.join(" "));
        out.push('\n');
        let lines: Vec<&str> = text.lines().collect();
        let mut shown_lines = 0usize;
        for (name, _kind, start, end) in frames.iter().take(3) {
            let s = (*start as usize).saturating_sub(1).min(lines.len());
            let e = (*end as usize).clamp(s + 1, lines.len());
            let slice = lines[s..e].join("\n");
            let body = compact_block(&slice, 16);
            out.push('\n');
            out.push_str(&format!("#{name}\n"));
            out.push_str(&body);
            out.push('\n');
            shown_lines += body.lines().count();
            if shown_lines > 48 {
                break;
            }
        }
        if frames.len() > 3 {
            out.push_str(&format!(
                "\n… {} more  ctx_fetch(\"{uri}#name\")\n",
                frames.len() - 3
            ));
        }
        return out;
    }
    out.push_str(&format!("ctx_fetch(\"{uri}\", q=…)  q=* full\n\n"));
    if tokens > FULL_PAGE_TOKENS {
        out.push_str(&signal_preview(text));
    }
    out
}

fn signal_preview(text: &str) -> String {
    ctx_optimizer::diagnostic_preview(text, 60, 36)
}

/// Score a blob against a search query. Higher is better.
pub fn match_score(text: &str, terms: &[&str]) -> u32 {
    if terms.is_empty() {
        return 0;
    }
    let terms: Vec<String> = terms.iter().map(|t| t.to_ascii_lowercase()).collect();
    let mut score = 0u32;
    for line in text.lines() {
        let l = line.to_ascii_lowercase();
        if terms.iter().any(|t| l.contains(t.as_str())) {
            score += 1;
        }
    }
    score
}

pub fn first_snippet(text: &str, terms: &[&str]) -> Option<String> {
    let terms: Vec<String> = terms.iter().map(|t| t.to_ascii_lowercase()).collect();
    let lines: Vec<&str> = text.lines().collect();
    let hit = lines.iter().position(|line| {
        let l = line.to_ascii_lowercase();
        terms.iter().any(|t| l.contains(t.as_str()))
    })?;
    let start = hit.saturating_sub(1);
    let end = (hit + 3).min(lines.len());
    let mut out = String::new();
    for (i, line) in lines[start..end].iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        out.push_str(&format!("{:>6} | {}", start + i + 1, line));
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_in_keeps_window_around_hit() {
        let mut text = String::new();
        for i in 0..40 {
            text.push_str(&format!("line {i} noise\n"));
        }
        text.push_str("secret token 401 here\n");
        for i in 41..80 {
            text.push_str(&format!("line {i} noise\n"));
        }
        let out = page_in(&text, "401");
        assert!(out.contains("401"), "{out}");
        assert!(!out.contains("line 0 noise"), "{out}");
    }

    #[test]
    fn frame_slice_is_a_page_walk() {
        let text = "a\nb\nFAIL auth\nc\nd\n";
        let out = frame_slice(text, "ctx://shell/ab", "auth", 3, 3);
        assert!(out.contains("FAIL auth"), "{out}");
        assert!(out.contains("ctx://shell/ab#auth"), "{out}");
    }

    #[test]
    fn miss_does_not_dump_full_page() {
        let out = page_in("hello world", "zzzz");
        assert!(out.contains("no match"), "{out}");
        assert!(out.contains("query=*"), "{out}");
        assert!(!out.contains("hello world"), "{out}");
    }

    #[test]
    fn small_preview_is_identity() {
        let text = "short page";
        assert_eq!(bounded_preview(text, "ctx://shell/ab"), text);
    }

    #[test]
    fn framed_preview_prefers_fail_bodies() {
        let mut text = String::from("running 80 tests\n");
        for i in 0..40 {
            text.push_str(&format!("test t{i} ... ok\n"));
        }
        text.push_str("---- auth::login stdout ----\nleft: 401\nright: 200\n");
        let frames = vec![("auth::login".into(), "fail".into(), 42u32, 44u32)];
        let out = bounded_preview_frames(&text, "ctx://shell/ab", &frames);
        assert!(out.contains("401"), "{out}");
        assert!(out.contains("#auth::login"), "{out}");
        assert!(!out.contains("test t12 ... ok"), "{out}");
    }

    #[test]
    fn search_terms_are_case_insensitive() {
        assert!(match_score("error: boom", &["ERROR"]) > 0);
        let snip = first_snippet("noise\nerror: boom\n", &["ERROR"]).unwrap();
        assert!(snip.contains("error: boom"), "{snip}");
    }
}
