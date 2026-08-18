//! Shared symbol extraction for file outlines and page-table frames.

const PREFIXES: &[&str] = &[
    "pub async fn ",
    "pub fn ",
    "async fn ",
    "fn ",
    "async def ",
    "def ",
    "class ",
    "export async function ",
    "export function ",
    "export class ",
    "function ",
    "pub struct ",
    "pub enum ",
    "pub trait ",
    "impl ",
    "func ",
    "interface ",
    "type ",
];

pub struct Symbol<'a> {
    pub keyword: &'a str,
    pub name: &'a str,
}

pub fn parse_symbol(t: &str) -> Option<Symbol<'_>> {
    for prefix in PREFIXES {
        let Some(rest) = t.strip_prefix(prefix) else {
            continue;
        };
        let name = rest
            .split(|c: char| !c.is_ascii_alphanumeric() && c != '_' && c != ':')
            .next()
            .unwrap_or("")
            .trim();
        if name.len() < 2 {
            continue;
        }
        let kind = prefix.trim();
        let keyword = kind
            .rsplit(' ')
            .next()
            .unwrap_or(kind)
            .trim_end_matches(" fn")
            .trim();
        return Some(Symbol { keyword, name });
    }
    None
}

pub fn symbol_name(t: &str) -> Option<&str> {
    parse_symbol(t).map(|s| s.name)
}

pub fn symbol_label(t: &str) -> Option<String> {
    parse_symbol(t).map(|s| format!("{} {}", s.keyword, s.name))
}
