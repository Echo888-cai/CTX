use crate::compact::compact_block;
use crate::pipeline::{OptimizeInput, OptimizeOutput, Optimizer};
use crate::symbols::{collect_symbol_spans, matching_spans, slice_span, SymbolSpan};
use crate::tokens::estimate_tokens;

pub struct ReadGuard;

impl Optimizer for ReadGuard {
    fn apply(&self, input: &OptimizeInput<'_>) -> Option<OptimizeOutput> {
        if input.kind != "file" {
            return None;
        }
        if input
            .metadata
            .get("unchanged")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            return Some(unchanged_stub(input));
        }
        let raw_tokens = input.raw_tokens;
        if raw_tokens < 400 {
            return None;
        }
        let path = input
            .metadata
            .get("path")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let task = task_tokens(input.metadata);
        let mut outline = outline_working_set(path, input.payload, &task);
        if let Some(window) = requested_window(input) {
            outline = format!("{window}\n\n{outline}");
        }
        let out = OptimizeOutput::reduced_terminal("file-read", outline);
        if raw_tokens.saturating_sub(out.delivered_tokens) < 80 {
            return None;
        }
        Some(out)
    }
}

fn task_tokens(metadata: &serde_json::Value) -> Vec<String> {
    metadata
        .get("task")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .split_whitespace()
        .filter(|t| t.len() >= 2)
        .map(|t| t.to_string())
        .collect()
}

fn requested_window(input: &OptimizeInput<'_>) -> Option<String> {
    let offset = input
        .metadata
        .get("offset")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let limit = input.metadata.get("limit").and_then(|v| v.as_u64())?;
    if limit == 0 {
        return None;
    }
    let lines: Vec<&str> = input.payload.lines().collect();
    if lines.is_empty() {
        return None;
    }
    let start = if offset <= 1 {
        0
    } else {
        (offset as usize - 1).min(lines.len())
    };
    let end = (start + limit as usize).min(lines.len());
    if end <= start {
        return None;
    }
    let mut out = format!("L{}–{}\n", start + 1, end);
    out.push_str(&lines[start..end].join("\n"));
    Some(out)
}

fn unchanged_stub(input: &OptimizeInput<'_>) -> OptimizeOutput {
    let path = input
        .metadata
        .get("path")
        .and_then(|v| v.as_str())
        .unwrap_or("file");
    let uri = input
        .metadata
        .get("uri")
        .and_then(|v| v.as_str())
        .unwrap_or("ctx://file/unknown");
    let regions = input
        .metadata
        .get("regions")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let mut text = format!("{path} unchanged  {uri}\n");
    if !regions.is_empty() {
        let bits: Vec<&str> = regions.iter().filter_map(|r| r.as_str()).take(8).collect();
        if !bits.is_empty() {
            text.push_str(&bits.join("  "));
            text.push('\n');
        }
    }
    let mut out = OptimizeOutput::reduced("file-read", text);
    out.terminal = true;
    out.duplicate_of = Some(uri.to_string());
    out
}

pub fn outline_source(path: &str, source: &str) -> String {
    outline_working_set(path, source, &[])
}

/// Signature table. Task-matching function bodies stay resident.
pub fn outline_working_set(path: &str, source: &str, task: &[String]) -> String {
    let spans = collect_symbol_spans(source);
    let total_lines = source.lines().count();
    let mut out = format!("{path}  {total_lines} lines\n");
    if !spans.is_empty() {
        out.push('\n');
        for s in spans.iter().take(48) {
            out.push_str(&format_sig(s));
            out.push('\n');
        }
    }
    append_task_bodies(&mut out, source, &spans, task);
    let q = spans.first().map(|s| s.name.as_str()).unwrap_or("");
    out.push_str(&format!("ctx_read path q={q}\n"));
    out
}

fn format_sig(s: &SymbolSpan) -> String {
    let sig = s.label();
    if s.start_line == s.end_line {
        format!("{sig}  L{}", s.start_line)
    } else {
        format!("{sig}  L{}–{}", s.start_line, s.end_line)
    }
}

fn append_task_bodies(out: &mut String, source: &str, spans: &[SymbolSpan], task: &[String]) {
    if task.is_empty() {
        return;
    }
    let hits = matching_spans(spans, task);
    if hits.is_empty() {
        return;
    }
    let mut extra = String::new();
    for s in hits.iter().take(3) {
        let body = slice_span(source, s.start_line, s.end_line);
        let compact = if body.lines().count() > 40 {
            compact_block(&body, 32)
        } else {
            body
        };
        extra.push('\n');
        extra.push_str(&format!("#{} L{}–{}\n", s.name, s.start_line, s.end_line));
        extra.push_str(&compact);
        extra.push('\n');
        if estimate_tokens(&extra) > 400 {
            break;
        }
    }
    out.push_str(&extra);
}

pub fn extract_regions(source: &str) -> Vec<String> {
    collect_symbol_spans(source)
        .into_iter()
        .take(24)
        .map(|s| {
            if s.start_line == s.end_line {
                format!("{}  L{}", s.label(), s.start_line)
            } else {
                format!("{}  L{}–{}", s.label(), s.start_line, s.end_line)
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::{OptimizeInput, Optimizer};
    use crate::tokens::estimate_tokens;

    #[test]
    fn outline_is_names_not_bodies() {
        let mut src = String::from("use std::io;\nuse std::fs;\n\n");
        for i in 0..80 {
            src.push_str(&format!("pub fn thing_{i}(x: i32) -> i32 {{ x + {i} }}\n"));
        }
        let out = outline_source("src/lib.rs", &src);
        assert!(out.contains("src/lib.rs"), "{out}");
        assert!(out.contains("fn thing_0"), "{out}");
        assert!(!out.contains("-> i32 { x +"), "{out}");
        assert!(estimate_tokens(&out) < estimate_tokens(&src) / 3, "{out}");
    }

    #[test]
    fn medium_file_is_outlined() {
        let mut src = String::from("use std::io;\n\n");
        for i in 0..50 {
            src.push_str(&format!("pub fn thing_{i}(x: i32) -> i32 {{ x + {i} }}\n"));
        }
        let input = OptimizeInput {
            kind: "file",
            tool_name: Some("Read"),
            payload: &src,
            metadata: &serde_json::json!({"path": "src/lib.rs"}),
            raw_tokens: estimate_tokens(&src),
        };
        assert!(input.raw_tokens > 400, "{}", input.raw_tokens);
        let out = ReadGuard.apply(&input).expect("outline medium file");
        assert!(out.text.contains("fn thing_0"), "{}", out.text);
        assert!(!out.text.contains("x + 12"), "{}", out.text);
        assert!(out.delivered_tokens + 80 < input.raw_tokens);
        assert!(out.terminal);
    }

    #[test]
    fn task_keeps_matching_body_resident() {
        let src = "\
fn noise() {\n\
    1\n\
}\n\
pub fn login(user: &str) -> i32 {\n\
    let status = 401;\n\
    status\n\
}\n\
fn other() {\n\
    2\n\
}\n";
        let mut padded = src.to_string();
        for i in 0..40 {
            padded.push_str(&format!("fn extra_{i}() {{ {i} }}\n"));
        }
        let out = outline_working_set("src/auth.rs", &padded, &["login".into()]);
        assert!(out.contains("let status = 401"), "{out}");
        assert!(out.contains("#login"), "{out}");
        assert!(!out.contains("fn noise() {\n    1"), "{out}");
    }
}
