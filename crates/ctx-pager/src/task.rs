//! Deterministic task tokens. No model.

use std::collections::HashSet;

const STOP: &[&str] = &[
    "a",
    "an",
    "the",
    "and",
    "or",
    "to",
    "of",
    "in",
    "on",
    "for",
    "with",
    "from",
    "this",
    "that",
    "please",
    "just",
    "can",
    "you",
    "we",
    "it",
    "is",
    "be",
    "at",
    "as",
    "by",
    "fix",
    "add",
    "make",
    "update",
    "implement",
    "change",
    "run",
    "check",
    "try",
    "test",
    "tests",
    "cargo",
    "npm",
    "yarn",
    "pnpm",
    "pytest",
    "jest",
    "nextest",
    "ctx",
    "exec",
    "shell",
    "bash",
    "sh",
    "zsh",
    "python",
    "python3",
    "node",
    "src",
    "lib",
    "mod",
    "use",
    "pub",
    "async",
    "fn",
    "impl",
    "struct",
    "error",
    "failed",
    "pass",
    "passed",
    "ok",
    "stdout",
    "stderr",
    "的",
    "了",
    "和",
    "或",
    "在",
    "是",
    "把",
    "被",
    "让",
    "请",
    "我",
    "你",
    "这",
    "那",
    "什么",
    "怎么",
    "可以",
    "一下",
    "这个",
    "那个",
    "我们",
    "帮我",
    "以及",
    "或者",
    "但是",
    "如果",
    "因为",
    "所以",
];

const DROP_EXT: &[&str] = &["rs", "py", "ts", "js", "go", "tsx", "jsx", "toml", "json"];

/// Split prompt / command / path / frame names into stable task tokens.
pub fn extract_task(parts: &[&str]) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for part in parts {
        for tok in split_tokens(part) {
            if !keep(&tok) {
                continue;
            }
            if seen.insert(tok.clone()) {
                out.push(tok);
            }
            if out.len() >= 16 {
                return out;
            }
        }
    }
    out
}

pub fn parse_task(s: &str) -> Vec<String> {
    extract_task(&[s])
}

pub fn format_task(tokens: &[String]) -> String {
    tokens.join(" ")
}

/// Merge task tokens. `new` wins: recent prompt/frame displaces stale ones.
pub fn merge_tokens(old: &[String], new: &[String]) -> Vec<String> {
    extract_task(
        &new.iter()
            .chain(old.iter())
            .map(String::as_str)
            .collect::<Vec<_>>(),
    )
}

/// How many query tokens hit this page's tokens. 0 = unrelated.
pub fn overlap(page: &[String], query: &[String]) -> u32 {
    if query.is_empty() || page.is_empty() {
        return 0;
    }
    let mut n = 0u32;
    for q in query {
        if page.iter().any(|p| token_hit(p, q)) {
            n += token_weight(q);
        }
    }
    n
}

fn token_weight(t: &str) -> u32 {
    match t.chars().count() {
        0..=2 => 1,
        3..=5 => 2,
        _ => 3,
    }
}

pub fn token_matches(page: &str, query: &str) -> bool {
    token_hit(page, query)
}

fn token_hit(page: &str, query: &str) -> bool {
    if page == query {
        return true;
    }
    let long = page.chars().count() >= 2 && query.chars().count() >= 2;
    long && (page.contains(query) || query.contains(page))
}

fn split_tokens(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut buf = String::new();
    let mut cjk = false;
    let mut latin = false;
    for c in s.chars() {
        if is_cjk(c) {
            if latin {
                flush_latin(&mut buf, &mut out);
                latin = false;
            }
            buf.push(c);
            cjk = true;
        } else if c.is_ascii_alphanumeric() {
            if cjk {
                flush_cjk(&mut buf, &mut out);
                cjk = false;
            }
            buf.push(c);
            latin = true;
        } else {
            if latin {
                flush_latin(&mut buf, &mut out);
                latin = false;
            }
            if cjk {
                flush_cjk(&mut buf, &mut out);
                cjk = false;
            }
        }
    }
    if latin {
        flush_latin(&mut buf, &mut out);
    }
    if cjk {
        flush_cjk(&mut buf, &mut out);
    }
    out
}

fn is_cjk(c: char) -> bool {
    let u = c as u32;
    (0x3400..=0x4DBF).contains(&u)
        || (0x4E00..=0x9FFF).contains(&u)
        || (0x3040..=0x30FF).contains(&u)
        || (0xAC00..=0xD7AF).contains(&u)
}

fn flush_latin(buf: &mut String, out: &mut Vec<String>) {
    if buf.is_empty() {
        return;
    }
    let pieces = split_camel(buf);
    let full = buf.to_ascii_lowercase();
    if pieces.len() > 1 {
        if full.chars().count() >= 3 {
            out.push(full);
        }
        for p in pieces {
            let l = p.to_ascii_lowercase();
            if l.chars().count() >= 2 {
                out.push(l);
            }
        }
    } else {
        out.push(full);
    }
    buf.clear();
}

fn flush_cjk(buf: &mut String, out: &mut Vec<String>) {
    if buf.is_empty() {
        return;
    }
    let chars: Vec<char> = buf.chars().collect();
    if chars.len() == 1 {
        out.push(buf.clone());
    } else {
        if chars.len() <= 6 {
            out.push(buf.clone());
        }
        for w in chars.windows(2) {
            out.push(w.iter().collect());
        }
    }
    buf.clear();
}

fn split_camel(s: &str) -> Vec<String> {
    let chars: Vec<char> = s.chars().collect();
    let mut parts = Vec::new();
    let mut cur = String::new();
    for (i, &c) in chars.iter().enumerate() {
        if c.is_ascii_uppercase() && i > 0 {
            let prev_lower = chars[i - 1].is_ascii_lowercase();
            let next_lower = chars.get(i + 1).is_some_and(|n| n.is_ascii_lowercase());
            if !cur.is_empty() && (prev_lower || (next_lower && chars[i - 1].is_ascii_uppercase()))
            {
                parts.push(std::mem::take(&mut cur));
            }
        }
        cur.push(c);
    }
    if !cur.is_empty() {
        parts.push(cur);
    }
    parts
}

fn keep(t: &str) -> bool {
    let chars = t.chars().count();
    if t.is_ascii() && chars < 3 {
        return t.starts_with('e') && t.chars().skip(1).all(|c| c.is_ascii_digit());
    }
    if !t.is_ascii() && chars < 2 {
        return false;
    }
    if STOP.contains(&t) || DROP_EXT.contains(&t) {
        return false;
    }
    if t.chars().all(|c| c.is_ascii_digit()) {
        return chars >= 3;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_keeps_substance() {
        let t = extract_task(&["fix the oauth redirect in auth login"]);
        assert!(t.contains(&"oauth".to_string()), "{t:?}");
        assert!(t.contains(&"redirect".to_string()), "{t:?}");
        assert!(t.contains(&"auth".to_string()), "{t:?}");
        assert!(t.contains(&"login".to_string()), "{t:?}");
        assert!(!t.iter().any(|x| x == "fix" || x == "the"), "{t:?}");
    }

    #[test]
    fn path_and_frame_yield_names() {
        let t = extract_task(&["src/auth.rs", "auth::login", "E0308"]);
        assert!(t.contains(&"auth".to_string()), "{t:?}");
        assert!(t.contains(&"login".to_string()), "{t:?}");
        assert!(t.contains(&"e0308".to_string()), "{t:?}");
        assert!(!t.contains(&"src".to_string()), "{t:?}");
        assert!(!t.contains(&"rs".to_string()), "{t:?}");
    }

    #[test]
    fn overlap_promotes_related_pages() {
        let page = extract_task(&["auth::login"]);
        let query = extract_task(&["fix auth"]);
        assert!(overlap(&page, &query) >= 1);
        let zh_page = extract_task(&["登录失败"]);
        let zh_query = extract_task(&["修复 登录"]);
        assert!(
            overlap(&zh_page, &zh_query) >= 1,
            "{zh_page:?} {zh_query:?}"
        );
        assert_eq!(overlap(&page, &extract_task(&["unrelated-crate"])), 0);
    }

    #[test]
    fn chinese_prompt_keeps_substance() {
        let t = extract_task(&["请修复登录和 oauth"]);
        assert!(t.iter().any(|x| x.contains("登录")), "{t:?}");
        assert!(t.contains(&"oauth".to_string()), "{t:?}");
        assert!(!t.iter().any(|x| x == "请" || x == "和"), "{t:?}");
    }

    #[test]
    fn chinese_without_spaces_yields_bigrams() {
        let t = extract_task(&["请修复登录"]);
        assert!(t.iter().any(|x| x.contains("登录")), "{t:?}");
        assert!(t.iter().any(|x| x.contains("修复")), "{t:?}");
    }

    #[test]
    fn camel_and_snake_split() {
        let t = extract_task(&["authLogin", "refresh_token"]);
        assert!(t.contains(&"login".to_string()), "{t:?}");
        assert!(t.contains(&"refresh".to_string()), "{t:?}");
        assert!(t.contains(&"token".to_string()), "{t:?}");
    }

    #[test]
    fn merge_prefers_new_tokens() {
        let old: Vec<String> = (0..16).map(|i| format!("old{i:02}xx")).collect();
        let new = extract_task(&["billing invoice"]);
        let merged = merge_tokens(&old, &new);
        assert!(merged.contains(&"billing".to_string()), "{merged:?}");
        assert!(merged.contains(&"invoice".to_string()), "{merged:?}");
        assert_eq!(merged.len(), 16, "{merged:?}");
        assert!(!merged.iter().any(|t| t == "old15xx"), "{merged:?}");
    }
}
