use crate::pipeline::{OptimizeInput, OptimizeOutput, Optimizer};

pub struct McpGuard;

impl Optimizer for McpGuard {
    fn apply(&self, input: &OptimizeInput<'_>) -> Option<OptimizeOutput> {
        if input.kind != "mcp" {
            return None;
        }
        if input.raw_tokens < 500 {
            return None;
        }
        let reduced = reduce_json_like(input.payload);
        let out = OptimizeOutput::reduced("mcp", reduced);
        if out.delivered_tokens + 80 >= input.raw_tokens {
            return None;
        }
        Some(out)
    }
}

const HOT_KEYS: &[&str] = &[
    "error",
    "errors",
    "message",
    "msg",
    "stderr",
    "stdout",
    "detail",
    "details",
    "traceback",
    "stack",
    "cause",
    "reason",
    "status",
    "code",
];

fn is_hot_key(k: &str) -> bool {
    HOT_KEYS.iter().any(|h| k.eq_ignore_ascii_case(h))
}

pub fn reduce_json_like(payload: &str) -> String {
    let trimmed = payload.trim();
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) {
        return render_json(&value, 0);
    }
    crate::generic::reduce_text(payload)
}

fn render_json(value: &serde_json::Value, depth: usize) -> String {
    match value {
        serde_json::Value::Array(items) => {
            let mut out = format!("[{}]\n", items.len());
            let preview = items.len().min(3);
            for (i, item) in items.iter().take(preview).enumerate() {
                out.push_str(&format!(
                    "[{i}] {}\n",
                    summarize_value(item, depth + 1, None)
                ));
            }
            if items.len() > preview {
                out.push_str(&format!("…{}\n", items.len() - preview));
            }
            if let Some(serde_json::Value::Object(map)) = items.first() {
                let keys: Vec<_> = map.keys().take(8).cloned().collect();
                out.push_str(&format!("keys {}\n", keys.join(",")));
            }
            out
        }
        serde_json::Value::Object(map) => {
            let mut out = format!("{{{}}}\n", map.len());
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort_by_key(|k| (!is_hot_key(k), k.as_str()));
            for k in keys.into_iter().take(24) {
                let v = &map[k];
                out.push_str(&format!(
                    "{k}: {}\n",
                    summarize_value(v, depth + 1, Some(k))
                ));
            }
            if map.len() > 24 {
                out.push_str(&format!("…{}\n", map.len() - 24));
            }
            out
        }
        other => summarize_value(other, depth, None),
    }
}

fn summarize_value(value: &serde_json::Value, depth: usize, key: Option<&str>) -> String {
    let hot = key.is_some_and(is_hot_key);
    match value {
        serde_json::Value::String(s) => {
            let cap = if hot { 400 } else { 80 };
            if s.len() > cap {
                format!(
                    "str({}c) {}…",
                    s.len(),
                    s.chars().take(cap / 2).collect::<String>()
                )
            } else {
                format!("\"{s}\"")
            }
        }
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Null => "null".into(),
        serde_json::Value::Array(a) => {
            if hot && depth < 2 && a.len() <= 8 {
                format!(
                    "[{}]",
                    a.iter()
                        .take(5)
                        .map(|v| summarize_value(v, depth + 1, None))
                        .collect::<Vec<_>>()
                        .join("; ")
                )
            } else {
                format!("[{}]", a.len())
            }
        }
        serde_json::Value::Object(m) => {
            if depth > 2 && !hot {
                format!("{{{}}}", m.len())
            } else {
                let keys: Vec<_> = m.keys().take(6).cloned().collect();
                format!("{{{}}} {}", m.len(), keys.join(","))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tokens::estimate_tokens;

    #[test]
    fn summarizes_big_array() {
        let items: Vec<_> = (0..200)
            .map(|i| serde_json::json!({"id": i, "body": "x".repeat(80)}))
            .collect();
        let raw = serde_json::to_string(&items).unwrap();
        let out = reduce_json_like(&raw);
        assert!(out.contains("[200]") || out.contains("200"), "{out}");
        assert!(estimate_tokens(&out) < estimate_tokens(&raw) / 5);
    }

    #[test]
    fn keeps_error_message() {
        let raw = serde_json::json!({
            "ok": true,
            "items": (0..50).map(|i| serde_json::json!({"id": i})).collect::<Vec<_>>(),
            "error": "redirect_uri mismatch at src/oauth.rs:192",
        })
        .to_string();
        let out = reduce_json_like(&raw);
        assert!(out.contains("redirect_uri mismatch"), "{out}");
        assert!(out.contains("src/oauth.rs"), "{out}");
    }
}
