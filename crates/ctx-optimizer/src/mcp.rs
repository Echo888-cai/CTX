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
        let out = OptimizeOutput::reduced_terminal("mcp", reduced);
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
        serde_json::Value::Array(items) => render_array(items, depth),
        serde_json::Value::Object(map) => render_object(map, depth),
        other => summarize_value(other, depth, None),
    }
}

fn looks_like_error_item(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Object(map) => map.keys().any(|k| {
            is_hot_key(k)
                || k.eq_ignore_ascii_case("path")
                || k.eq_ignore_ascii_case("file")
                || k.eq_ignore_ascii_case("loc")
                || k.eq_ignore_ascii_case("line")
        }),
        serde_json::Value::String(s) => {
            let l = s.to_ascii_lowercase();
            l.contains("error") || l.contains("fail") || l.contains("exception")
        }
        _ => false,
    }
}

fn error_line(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => s.chars().take(160).collect(),
        serde_json::Value::Object(map) => {
            let msg = ["message", "msg", "error", "detail", "reason", "title"]
                .iter()
                .find_map(|k| map.iter().find(|(ik, _)| ik.eq_ignore_ascii_case(k)))
                .and_then(|(_, v)| v.as_str())
                .unwrap_or("");
            let path = ["path", "file", "filename", "loc"]
                .iter()
                .find_map(|k| map.iter().find(|(ik, _)| ik.eq_ignore_ascii_case(k)))
                .and_then(|(_, v)| v.as_str())
                .unwrap_or("");
            let line = map
                .get("line")
                .or_else(|| map.get("lineno"))
                .and_then(|v| v.as_u64());
            match (path.is_empty(), msg.is_empty(), line) {
                (false, false, Some(n)) => format!("{path}:{n}  {msg}"),
                (false, false, None) => format!("{path}  {msg}"),
                (false, true, _) => path.to_string(),
                (true, false, _) => msg.chars().take(160).collect(),
                _ => summarize_value(value, 2, None),
            }
        }
        other => summarize_value(other, 2, None),
    }
}

fn render_error_list(items: &[serde_json::Value]) -> String {
    let mut out = format!("[{}]\n", items.len());
    for item in items.iter().take(16) {
        out.push_str(&error_line(item));
        out.push('\n');
    }
    if items.len() > 16 {
        out.push_str(&format!("… {} more\n", items.len() - 16));
    }
    out
}

fn render_array(items: &[serde_json::Value], depth: usize) -> String {
    let errorish = items.iter().filter(|i| looks_like_error_item(i)).count();
    if !items.is_empty() && errorish * 2 >= items.len() {
        return render_error_list(items);
    }
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

fn render_object(map: &serde_json::Map<String, serde_json::Value>, depth: usize) -> String {
    if let Some((key, items)) = named_error_array(map) {
        let mut out = format!("{{{}}}  {key}\n", map.len());
        for k in ["status", "code", "message", "msg"] {
            if let Some((ik, v)) = map.iter().find(|(ik, _)| ik.eq_ignore_ascii_case(k)) {
                if !v.is_array() {
                    out.push_str(&format!(
                        "{ik}: {}\n",
                        summarize_value(v, depth + 1, Some(ik))
                    ));
                }
            }
        }
        out.push_str(&render_error_list(items));
        return out;
    }
    let mut out = format!("{{{}}}\n", map.len());
    let mut keys: Vec<&String> = map.keys().collect();
    keys.sort_by_key(|k| (!is_hot_key(k), k.as_str()));
    for k in keys.into_iter().take(24) {
        let v = &map[k];
        if let Some(arr) = v.as_array() {
            if error_list_ratio(arr) {
                out.push_str(k);
                out.push('\n');
                out.push_str(&render_error_list(arr));
                continue;
            }
        }
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

fn named_error_array(
    map: &serde_json::Map<String, serde_json::Value>,
) -> Option<(&str, &[serde_json::Value])> {
    for name in ["errors", "issues", "diagnostics", "violations"] {
        if let Some(arr) = map
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .and_then(|(_, v)| v.as_array())
        {
            if error_list_ratio(arr) {
                return Some((name, arr.as_slice()));
            }
        }
    }
    None
}

fn error_list_ratio(items: &[serde_json::Value]) -> bool {
    !items.is_empty()
        && items.iter().filter(|i| looks_like_error_item(i)).count() * 2 >= items.len()
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

    #[test]
    fn error_array_keeps_each_message() {
        let items: Vec<_> = (0..12)
            .map(|i| {
                serde_json::json!({
                    "path": format!("src/mod{i}.rs"),
                    "line": 10 + i,
                    "message": format!("redirect_uri mismatch {i} {}", "detail ".repeat(20))
                })
            })
            .collect();
        let raw = serde_json::to_string(&items).unwrap();
        let out = reduce_json_like(&raw);
        assert!(out.contains("redirect_uri mismatch 0"), "{out}");
        assert!(out.contains("redirect_uri mismatch 7"), "{out}");
        assert!(out.contains("src/mod0.rs"), "{out}");
        assert!(out.matches("redirect_uri mismatch").count() >= 8, "{out}");
        assert!(!out.contains("\"path\":"), "{out}");
    }
}
