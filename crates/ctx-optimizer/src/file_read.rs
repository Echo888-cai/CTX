use crate::pipeline::{OptimizeInput, OptimizeOutput, Optimizer};
use crate::symbols::symbol_label;

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
        let outline = outline_source(path, input.payload);
        let out = OptimizeOutput::reduced("file-read", outline);
        if raw_tokens.saturating_sub(out.delivered_tokens) < 80 {
            return None;
        }
        Some(out)
    }
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
    let total_lines = source.lines().count();
    let mut sigs = Vec::new();
    for (i, line) in source.lines().enumerate() {
        let t = line.trim();
        if let Some(name) = symbol_label(t) {
            sigs.push(format!("L{} {name}", i + 1));
        }
    }
    let mut out = format!("{path}  {total_lines} lines\n");
    let substance: Vec<&str> = source
        .lines()
        .filter(|l| {
            let t = l.trim();
            !t.is_empty()
                && !t.starts_with("use ")
                && !t.starts_with("import ")
                && !t.starts_with("from ")
                && !t.starts_with('#')
                && !t.starts_with("//")
                && !t.starts_with("/*")
                && t != "{"
                && t != "}"
                && symbol_label(t).is_none()
        })
        .take(8)
        .collect();
    if !substance.is_empty() {
        out.push('\n');
        out.push_str(&substance.join("\n"));
        out.push('\n');
    }
    if !sigs.is_empty() {
        out.push('\n');
        for s in sigs.iter().take(40) {
            out.push_str(s);
            out.push('\n');
        }
    }
    out.push_str("ctx_read path q=\n");
    out
}

pub fn extract_regions(source: &str) -> Vec<String> {
    let mut regions = Vec::new();
    for (i, line) in source.lines().enumerate() {
        let t = line.trim();
        if let Some(name) = symbol_label(t) {
            regions.push(format!("L{} {name}", i + 1));
        }
    }
    regions
}

#[cfg(test)]
mod tests {
    use super::*;
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
    }
}
