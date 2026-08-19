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

#[derive(Debug, Clone)]
pub struct SymbolSpan {
    pub name: String,
    pub keyword: String,
    pub start_line: u32,
    pub end_line: u32,
    pub signature: String,
}

impl SymbolSpan {
    pub fn label(&self) -> String {
        format!("{} {}", self.keyword, self.name)
    }
}

pub fn parse_symbol(t: &str) -> Option<Symbol<'_>> {
    let t = strip_visibility(t);
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

fn strip_visibility(t: &str) -> &str {
    t.strip_prefix("pub(crate) ")
        .or_else(|| t.strip_prefix("pub(super) "))
        .or_else(|| t.strip_prefix("pub(self) "))
        .unwrap_or(t)
}

pub fn symbol_name(t: &str) -> Option<&str> {
    parse_symbol(t).map(|s| s.name)
}

pub fn symbol_label(t: &str) -> Option<String> {
    parse_symbol(t).map(|s| format!("{} {}", s.keyword, s.name))
}

/// Named ranges covering each symbol's body, not just the declaration line.
pub fn collect_symbol_spans(source: &str) -> Vec<SymbolSpan> {
    if uses_indent(source) {
        collect_indent_spans(source)
    } else {
        collect_brace_spans(source)
    }
}

fn uses_indent(source: &str) -> bool {
    source.lines().any(|l| {
        let t = l.trim();
        t.starts_with("def ") || t.starts_with("async def ")
    })
}

fn signature_of(line: &str) -> String {
    let t = line.trim();
    let cut = t.find('{').or_else(|| t.find(':')).unwrap_or(t.len());
    let s = t[..cut].trim();
    if s.chars().count() > 80 {
        format!("{}…", s.chars().take(79).collect::<String>())
    } else {
        s.to_string()
    }
}

fn collect_brace_spans(source: &str) -> Vec<SymbolSpan> {
    let mut out = Vec::new();
    let mut open: Vec<(SymbolSpan, i32)> = Vec::new();
    let mut depth = 0i32;
    for (i, line) in source.lines().enumerate() {
        let n = (i + 1) as u32;
        let depth_before = depth;
        let t = line.trim();
        let sym = parse_symbol(t);
        depth = (depth + brace_delta(line)).max(0);

        if let Some(sym) = sym {
            let span = SymbolSpan {
                name: sym.name.to_string(),
                keyword: sym.keyword.to_string(),
                start_line: n,
                end_line: n,
                signature: signature_of(line),
            };
            if depth > depth_before || looks_unclosed_header(line) {
                open.push((span, depth_before));
            } else {
                out.push(span);
            }
        }

        while let Some((span, target)) = open.last() {
            if depth <= *target && span.start_line < n {
                let (mut done, _) = open.pop().expect("open");
                done.end_line = n;
                out.push(done);
            } else {
                break;
            }
        }
    }
    for mut span in open.into_iter().map(|(s, _)| s) {
        span.end_line = span
            .end_line
            .max(source.lines().count() as u32)
            .max(span.start_line);
        out.push(span);
    }
    out.sort_by_key(|s| s.start_line);
    out
}

fn looks_unclosed_header(line: &str) -> bool {
    let code = strip_line_comment(line);
    !code.contains('{') && !code.trim_end().ends_with(';')
}

fn collect_indent_spans(source: &str) -> Vec<SymbolSpan> {
    let mut out = Vec::new();
    let mut open: Vec<(SymbolSpan, usize)> = Vec::new();
    let mut last_content = 0u32;
    let lines: Vec<&str> = source.lines().collect();
    for (i, line) in lines.iter().enumerate() {
        let n = (i + 1) as u32;
        if is_blank_or_comment(line) {
            continue;
        }
        let ind = indent_of(line);
        let t = line.trim();
        while open.last().is_some_and(|(_, base)| ind <= *base) {
            let (mut done, _) = open.pop().expect("open");
            done.end_line = last_content.max(done.start_line);
            out.push(done);
        }
        last_content = n;
        if let Some(sym) = parse_symbol(t) {
            open.push((
                SymbolSpan {
                    name: sym.name.to_string(),
                    keyword: sym.keyword.to_string(),
                    start_line: n,
                    end_line: n,
                    signature: signature_of(line),
                },
                ind,
            ));
        }
    }
    for mut span in open.into_iter().map(|(s, _)| s) {
        span.end_line = last_content.max(span.start_line);
        out.push(span);
    }
    out.sort_by_key(|s| s.start_line);
    out
}

fn indent_of(line: &str) -> usize {
    let mut n = 0usize;
    for c in line.chars() {
        match c {
            ' ' => n += 1,
            '\t' => n += 4,
            _ => break,
        }
    }
    n
}

fn is_blank_or_comment(line: &str) -> bool {
    let t = line.trim();
    t.is_empty() || t.starts_with('#') || t.starts_with("//")
}

fn strip_line_comment(line: &str) -> &str {
    let mut in_str = false;
    let bytes = line.as_bytes();
    let mut i = 0usize;
    while i + 1 < bytes.len() {
        let c = bytes[i];
        if c == b'"' {
            in_str = !in_str;
        } else if !in_str && c == b'/' && bytes[i + 1] == b'/' {
            return &line[..i];
        }
        i += 1;
    }
    line
}

fn brace_delta(line: &str) -> i32 {
    let code = strip_line_comment(line);
    let mut delta = 0i32;
    let mut in_str = false;
    let mut escape = false;
    for c in code.chars() {
        if escape {
            escape = false;
            continue;
        }
        if in_str {
            if c == '\\' {
                escape = true;
            } else if c == '"' {
                in_str = false;
            }
            continue;
        }
        match c {
            '"' => in_str = true,
            '{' => delta += 1,
            '}' => delta -= 1,
            _ => {}
        }
    }
    delta
}

fn span_hits_task(span: &SymbolSpan, task: &[String]) -> bool {
    if task.is_empty() {
        return false;
    }
    let name = span.name.to_ascii_lowercase();
    task.iter().any(|t| {
        let t = t.to_ascii_lowercase();
        t.len() >= 2 && (name == t || name.contains(&t) || t.contains(&name))
    })
}

/// Slice source lines for a 1-indexed inclusive range.
/// Tightest symbol whose span covers `line` (1-based).
pub fn symbol_at_line(source: &str, line: u32) -> Option<String> {
    collect_symbol_spans(source)
        .into_iter()
        .filter(|s| s.start_line <= line && line <= s.end_line)
        .min_by_key(|s| s.end_line.saturating_sub(s.start_line))
        .map(|s| s.name)
}

pub fn slice_span(source: &str, start_line: u32, end_line: u32) -> String {
    let start = start_line.saturating_sub(1) as usize;
    source
        .lines()
        .skip(start)
        .take(end_line.saturating_sub(start_line).saturating_add(1) as usize)
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn matching_spans<'a>(spans: &'a [SymbolSpan], task: &[String]) -> Vec<&'a SymbolSpan> {
    spans.iter().filter(|s| span_hits_task(s, task)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rust_fn_covers_the_body() {
        let src = "\
fn other() {\n\
    1\n\
}\n\
pub fn login(x: i32) -> i32 {\n\
    let y = x + 1;\n\
    y\n\
}\n\
fn tail() {}\n";
        let spans = collect_symbol_spans(src);
        let login = spans.iter().find(|s| s.name == "login").expect("login");
        assert_eq!(login.start_line, 4, "{spans:?}");
        assert_eq!(login.end_line, 7, "{login:?}");
        let body = slice_span(src, login.start_line, login.end_line);
        assert!(body.contains("let y = x + 1"), "{body}");
        assert!(!body.contains("fn other"), "{body}");
    }

    #[test]
    fn one_liner_stays_one_line() {
        let src = "pub fn thing_0(x: i32) -> i32 { x + 0 }\n";
        let spans = collect_symbol_spans(src);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].start_line, spans[0].end_line);
    }

    #[test]
    fn python_def_uses_indent() {
        let src = concat!(
            "def other():\n",
            "    return 1\n",
            "\n",
            "def login(x):\n",
            "    y = x + 1\n",
            "    return y\n",
            "\n",
            "def tail():\n",
            "    pass\n",
        );
        let spans = collect_symbol_spans(src);
        let login = spans.iter().find(|s| s.name == "login").expect("login");
        assert_eq!(login.start_line, 4, "{spans:?}");
        assert_eq!(login.end_line, 6, "{login:?} src={src:?}");
    }

    #[test]
    fn impl_and_methods_are_nested_ranges() {
        let src = "\
impl Foo {\n\
    fn bar() {\n\
        1\n\
    }\n\
    fn baz() {\n\
        2\n\
    }\n\
}\n";
        let spans = collect_symbol_spans(src);
        let foo = spans.iter().find(|s| s.name == "Foo").expect("Foo");
        let bar = spans.iter().find(|s| s.name == "bar").expect("bar");
        assert_eq!(foo.start_line, 1);
        assert_eq!(foo.end_line, 8, "{foo:?}");
        assert_eq!(bar.start_line, 2);
        assert_eq!(bar.end_line, 4, "{bar:?}");
    }
}
