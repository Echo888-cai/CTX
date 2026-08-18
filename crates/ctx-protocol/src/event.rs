use serde::{Deserialize, Serialize};

use crate::{Harness, ToolRef};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EventKind {
    #[serde(rename = "tool.output", alias = "tool_output")]
    ToolOutput,
    #[serde(rename = "tool.input", alias = "tool_input")]
    ToolInput,
    #[serde(rename = "file.read", alias = "file_read")]
    FileRead,
    #[serde(rename = "prompt.submit", alias = "prompt_submit")]
    PromptSubmit,
    #[serde(
        rename = "session.start",
        alias = "session_start",
        alias = "SessionStart"
    )]
    SessionStart,
    #[serde(rename = "session.end", alias = "session_end", alias = "SessionEnd")]
    SessionEnd,
    #[serde(rename = "compact")]
    Compact,
}

impl EventKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ToolOutput => "tool.output",
            Self::ToolInput => "tool.input",
            Self::FileRead => "file.read",
            Self::PromptSubmit => "prompt.submit",
            Self::SessionStart => "session.start",
            Self::SessionEnd => "session.end",
            Self::Compact => "compact",
        }
    }

    pub fn from_hook_name(name: &str) -> Self {
        match name {
            "PostToolUse"
            | "postToolUse"
            | "afterShellExecution"
            | "afterMCPExecution"
            | "PostToolUseFailure"
            | "postToolUseFailure" => Self::ToolOutput,
            "PreToolUse" | "preToolUse" | "beforeShellExecution" | "beforeMCPExecution" => {
                Self::ToolInput
            }
            "beforeReadFile" | "Read" => Self::FileRead,
            "UserPromptSubmit" | "beforeSubmitPrompt" => Self::PromptSubmit,
            "SessionStart" | "sessionStart" => Self::SessionStart,
            "SessionEnd" | "sessionEnd" | "stop" => Self::SessionEnd,
            "PreCompact" | "preCompact" | "PostCompact" | "postCompact" => Self::Compact,
            _ => Self::ToolOutput,
        }
    }
}

/// Unified event ingested by the CTX runtime.
///
/// Harness adapters must convert into this shape. Core does not know
/// Claude `PostToolUse` from Cursor `postToolUse`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CtxEvent {
    pub event: EventKind,
    pub session: String,
    pub harness: Harness,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<ToolRef>,
    #[serde(default)]
    pub payload: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_context: Option<String>,
    #[serde(default = "default_meta")]
    pub metadata: serde_json::Value,
}

fn default_meta() -> serde_json::Value {
    serde_json::json!({})
}

impl CtxEvent {
    pub fn tool_output(
        session: impl Into<String>,
        harness: Harness,
        tool: ToolRef,
        payload: impl Into<String>,
    ) -> Self {
        Self {
            event: EventKind::ToolOutput,
            session: session.into(),
            harness,
            tool: Some(tool),
            payload: payload.into(),
            task_context: None,
            metadata: serde_json::json!({}),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spec_json_roundtrip() {
        let event = CtxEvent {
            event: EventKind::ToolOutput,
            session: "s1".into(),
            harness: Harness::ClaudeCode,
            tool: Some(ToolRef::new("Bash")),
            payload: "hello".into(),
            task_context: Some("fix auth".into()),
            metadata: serde_json::json!({"cwd": "/tmp"}),
        };
        let v = serde_json::to_value(&event).unwrap();
        assert_eq!(v["event"], "tool.output");
        assert_eq!(v["harness"], "claude-code");
        let back: CtxEvent = serde_json::from_value(v).unwrap();
        assert_eq!(back.payload, "hello");
    }
}
