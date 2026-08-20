//! Five frozen tools. Foreign MCP schemas stay behind `ctx.exec`.

use serde_json::{json, Value};

pub const PROTOCOL_VERSION: &str = "ctx-protocol-v7";
pub const TOOLS_VERSION: &str = "ctx-runtime-v4";

pub fn protocol_text() -> &'static str {
    "CTX protocol v7. Tools: ctx_search, ctx_fetch, ctx_exec, ctx_apply, ctx_inspect. Schemas frozen."
}

pub fn tools() -> Vec<Value> {
    vec![
        tool(
            "ctx_search",
            "Find stored pages, capabilities, or repo symbols.",
            json!({"query": {"type": "string"}, "limit": {"type": "integer"}}),
            &["query"],
        ),
        tool(
            "ctx_fetch",
            "Page in a ctx:// URI or capability handle.",
            json!({"uri": {"type": "string"}, "query": {"type": "string"}}),
            &["uri"],
        ),
        tool(
            "ctx_exec",
            "Run a capability:// handle. Schema lives in CTX, not in this tool list.",
            json!({"handle": {"type": "string"}, "arguments": {"type": "object"}}),
            &["handle"],
        ),
        tool(
            "ctx_apply",
            "Apply a patch or write through CTX (copy-on-write overlay).",
            json!({"path": {"type": "string"}, "contents": {"type": "string"}}),
            &["path"],
        ),
        tool(
            "ctx_inspect",
            "Show HOT/WARM/COLD working set and the current epoch.",
            json!({"session": {"type": "string"}}),
            &[],
        ),
    ]
}

fn tool(name: &str, description: &str, properties: Value, required: &[&str]) -> Value {
    json!({
        "name": name,
        "description": description,
        "input_schema": {
            "type": "object",
            "properties": properties,
            "required": required
        }
    })
}

pub fn tools_hash() -> String {
    crate::canonical::prefix_hash(&[
        PROTOCOL_VERSION,
        TOOLS_VERSION,
        &serde_json::to_string(&tools()).unwrap_or_default(),
    ])
}

/// Replace a large tool list with the frozen CTX surface.
pub fn wrap_tools(original: &[Value]) -> Vec<Value> {
    let _ = original;
    tools()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn five_frozen_tools() {
        assert_eq!(tools().len(), 5);
        assert_eq!(tools_hash(), tools_hash());
    }
}
