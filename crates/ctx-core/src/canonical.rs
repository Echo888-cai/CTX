//! Prompt canonicalizer — kill cache misses caused by JSON key order, clocks, UUIDs.

use serde_json::{Map, Value};

use ctx_store::blake3_hex;

pub fn canonicalize_json(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().filter(|k| !is_volatile_key(k)).collect();
            keys.sort();
            let mut out = Map::new();
            for k in keys {
                out.insert(k.clone(), canonicalize_json(&map[k]));
            }
            Value::Object(out)
        }
        Value::Array(arr) => Value::Array(arr.iter().map(canonicalize_json).collect()),
        Value::String(s) => Value::String(strip_clocks(s)),
        other => other.clone(),
    }
}

pub fn canonicalize_text(text: &str) -> String {
    let t = text.replace("\r\n", "\n").replace('\r', "\n");
    if let Ok(v) = serde_json::from_str::<Value>(&t) {
        return serde_json::to_string(&canonicalize_json(&v)).unwrap_or(t);
    }
    strip_clocks(&t)
}

pub fn prefix_hash(parts: &[&str]) -> String {
    let joined = parts
        .iter()
        .map(|p| canonicalize_text(p))
        .collect::<Vec<_>>()
        .join("\n\x1e\n");
    blake3_hex(joined.as_bytes())
}

fn is_volatile_key(k: &str) -> bool {
    matches!(
        k,
        "timestamp"
            | "generated_at"
            | "now"
            | "date"
            | "time"
            | "current_time"
            | "request_id"
            | "uuid"
            | "nonce"
            | "sessionStartTime"
    )
}

fn strip_clocks(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if looks_rfc3339(&b[i..]) {
            out.push_str("<ts>");
            i += rfc3339_len(&b[i..]);
            continue;
        }
        if looks_uuid(&b[i..]) {
            out.push_str("<id>");
            i += 36;
            continue;
        }
        out.push(b[i] as char);
        i += 1;
    }
    out
}

fn looks_rfc3339(b: &[u8]) -> bool {
    // 2026-08-20T10:00:00
    b.len() >= 19
        && b[0].is_ascii_digit()
        && b[1].is_ascii_digit()
        && b[2].is_ascii_digit()
        && b[3].is_ascii_digit()
        && b[4] == b'-'
        && b[10] == b'T'
        && b[13] == b':'
        && b[16] == b':'
}

fn rfc3339_len(b: &[u8]) -> usize {
    let mut n = 19;
    if n < b.len() && b[n] == b'.' {
        n += 1;
        while n < b.len() && b[n].is_ascii_digit() {
            n += 1;
        }
    }
    if n < b.len() && (b[n] == b'Z' || b[n] == b'z') {
        n += 1;
    }
    n
}

fn looks_uuid(b: &[u8]) -> bool {
    if b.len() < 36 {
        return false;
    }
    let hex = |c: u8| c.is_ascii_hexdigit();
    for (i, c) in b[..36].iter().enumerate() {
        if i == 8 || i == 13 || i == 18 || i == 23 {
            if *c != b'-' {
                return false;
            }
        } else if !hex(*c) {
            return false;
        }
    }
    true
}

/// Anthropic-style `cache_control` on the last stable system block.
pub fn with_cache_breakpoint(system: &Value) -> Value {
    let mut block = serde_json::json!({
        "type": "text",
        "text": "",
        "cache_control": { "type": "ephemeral" }
    });
    match system {
        Value::String(s) => {
            block["text"] = Value::String(canonicalize_text(s));
            serde_json::json!([block])
        }
        Value::Array(arr) => {
            let mut out: Vec<Value> = arr.iter().map(canonicalize_json).collect();
            if let Some(last) = out.last_mut() {
                if last.is_object() {
                    last.as_object_mut()
                        .unwrap()
                        .insert("cache_control".into(), serde_json::json!({ "type": "ephemeral" }));
                }
            }
            Value::Array(out)
        }
        other => {
            block["text"] = canonicalize_json(other);
            serde_json::json!([block])
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn sorts_keys_and_drops_clocks() {
        let a = json!({"b": 1, "a": 2, "timestamp": "now"});
        let c = canonicalize_json(&a);
        let s = serde_json::to_string(&c).unwrap();
        assert_eq!(s, r#"{"a":2,"b":1}"#);
    }

    #[test]
    fn same_prompt_same_hash() {
        let h1 = prefix_hash(&["hello 2026-08-20T10:00:00Z", r#"{"z":1,"a":2}"#]);
        let h2 = prefix_hash(&["hello 2026-08-21T11:11:11Z", r#"{"a":2,"z":1}"#]);
        assert_eq!(h1, h2);
    }
}
