//! Ripgrep / grep / find listings.
//!
//! Agent search dumps are the other big token leak besides tests: hundreds of
//! `path:line:text` rows, most of them the same file. Keep a per-file sample
//! and the counts. Directory listings keep a prefix plus a total.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchKind {
    Grep,
    Listing,
}

pub fn detect(text: &str, command: Option<&str>) -> Option<SearchKind> {
    if looks_like_tests(text) {
        return None;
    }
    if let Some(cmd) = command {
        if looks_like_grep_command(cmd) {
            return Some(SearchKind::Grep);
        }
        if looks_like_listing_command(cmd) && text.lines().count() >= 24 {
            return Some(SearchKind::Listing);
        }
    }
    if looks_like_grep(text) {
        return Some(SearchKind::Grep);
    }
    if looks_like_listing(text) {
        return Some(SearchKind::Listing);
    }
    None
}

pub fn reduce(text: &str, kind: SearchKind) -> String {
    match kind {
        SearchKind::Grep => reduce_grep(text),
        SearchKind::Listing => reduce_listing(text),
    }
}

fn looks_like_tests(text: &str) -> bool {
    text.contains("test result:")
        || text.contains("error[E")
        || text.contains("short test summary info")
        || text.contains("Test Files")
}

fn looks_like_grep_command(cmd: &str) -> bool {
    let c = cmd.to_ascii_lowercase();
    let needles = [
        "rg ", "rg\t", "ripgrep ", "grep ", "egrep ", "fgrep ", "git grep", "ag ", "ack ",
    ];
    needles.iter().any(|n| c.contains(n)) || c == "rg" || c.ends_with(" rg") || c.starts_with("rg ")
}

fn looks_like_listing_command(cmd: &str) -> bool {
    let c = cmd.trim().to_ascii_lowercase();
    c.starts_with("find ")
        || c.starts_with("ls ")
        || c.starts_with("tree ")
        || c == "ls"
        || c == "tree"
        || c == "find"
        || c.contains(" && ls ")
}

fn looks_like_grep(text: &str) -> bool {
    let mut n = 0u32;
    for line in text.lines() {
        if parse_hit(line).is_some() {
            n += 1;
        }
        if n >= 8 {
            return true;
        }
    }
    false
}

fn looks_like_listing(text: &str) -> bool {
    let mut paths = 0u32;
    let mut other = 0u32;
    for line in text.lines() {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        if looks_like_path_line(t) {
            paths += 1;
        } else {
            other += 1;
        }
    }
    paths >= 32 && paths > other * 3
}

fn looks_like_path_line(t: &str) -> bool {
    if t.contains("://") {
        return false;
    }
    let lower = t.to_ascii_lowercase();
    t.contains('/')
        || lower.ends_with(".rs")
        || lower.ends_with(".py")
        || lower.ends_with(".ts")
        || lower.ends_with(".js")
        || lower.ends_with(".go")
        || lower.ends_with(".tsx")
}

#[derive(Clone)]
struct Hit {
    path: String,
    line: Option<u32>,
    text: String,
}

fn parse_hit(line: &str) -> Option<Hit> {
    let t = line.trim_end();
    if t.is_empty() {
        return None;
    }
    // rg context: path-12-text
    if let Some(h) = parse_sep(t, '-') {
        if h.line.is_some() {
            return Some(h);
        }
    }
    parse_sep(t, ':')
}

fn parse_sep(t: &str, sep: char) -> Option<Hit> {
    let (path, rest) = t.split_once(sep)?;
    if path.len() < 3 || path.contains(' ') {
        return None;
    }
    let lower = path.to_ascii_lowercase();
    let looks_path = path.contains('/')
        || lower.ends_with(".rs")
        || lower.ends_with(".py")
        || lower.ends_with(".ts")
        || lower.ends_with(".js")
        || lower.ends_with(".go")
        || lower.ends_with(".tsx")
        || lower.ends_with(".jsx");
    if !looks_path {
        return None;
    }
    let rest = rest.strip_prefix(sep).unwrap_or(rest);
    let (num, text) = match rest.split_once(sep) {
        Some((n, txt)) => (n, txt),
        None => (rest, ""),
    };
    let line = num.parse::<u32>().ok();
    if line.is_none() && sep == '-' {
        return None;
    }
    Some(Hit {
        path: path.replace('\\', "/"),
        line,
        text: text.trim().chars().take(120).collect(),
    })
}

fn reduce_grep(text: &str) -> String {
    let mut files: Vec<(String, Vec<Hit>)> = Vec::new();
    let mut extra_files = 0u32;
    let mut total = 0u32;
    for line in text.lines() {
        let Some(hit) = parse_hit(line) else {
            continue;
        };
        total += 1;
        if let Some((_, rows)) = files.iter_mut().find(|(p, _)| *p == hit.path) {
            if rows.len() < 4 {
                rows.push(hit);
            }
            continue;
        }
        if files.len() >= 8 {
            extra_files += 1;
            continue;
        }
        files.push((hit.path.clone(), vec![hit]));
    }
    if files.is_empty() {
        return crate::compact::diagnostic_excerpt(text, 24);
    }
    let mut body = format!(
        "{} matches, {} files\n",
        total,
        files.len() as u32 + extra_files
    );
    for (path, rows) in &files {
        body.push_str(path);
        body.push('\n');
        for h in rows {
            match h.line {
                Some(n) => body.push_str(&format!("  {n}: {}\n", h.text)),
                None => body.push_str(&format!("  {}\n", h.text)),
            }
        }
    }
    if extra_files > 0 {
        body.push_str(&format!("… {extra_files} more files\n"));
    }
    body
}

fn reduce_listing(text: &str) -> String {
    let mut paths = Vec::new();
    for line in text.lines() {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        paths.push(t);
    }
    let n = paths.len();
    let mut body = format!("{n} paths\n");
    for p in paths.iter().take(24) {
        body.push_str(p);
        body.push('\n');
    }
    if n > 24 {
        body.push_str(&format!("… {} more\n", n - 24));
    }
    body
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tokens::estimate_tokens;

    #[test]
    fn rg_groups_by_file() {
        let mut raw = String::new();
        for i in 0..30 {
            raw.push_str(&format!("src/auth.rs:{i}: let status = {i}\n"));
        }
        for i in 0..20 {
            raw.push_str(&format!("src/other.rs:{i}: noise {i}\n"));
        }
        let out = reduce_grep(&raw);
        assert!(out.contains("src/auth.rs"), "{out}");
        assert!(out.contains("matches"), "{out}");
        assert!(!out.contains("let status = 20"), "{out}");
        assert!(estimate_tokens(&out) < estimate_tokens(&raw) / 3, "{out}");
    }

    #[test]
    fn find_is_a_prefix() {
        let mut raw = String::new();
        for i in 0..80 {
            raw.push_str(&format!("./src/mod{i}.rs\n"));
        }
        let out = reduce_listing(&raw);
        assert!(out.contains("80 paths"), "{out}");
        assert!(out.contains("./src/mod0.rs"), "{out}");
        assert!(!out.contains("./src/mod50.rs"), "{out}");
    }
}
