use serde::{Deserialize, Serialize};

/// Which AI harness produced an event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Harness {
    #[serde(alias = "claude")]
    ClaudeCode,
    Cursor,
    #[serde(other)]
    Unknown,
}

impl Harness {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ClaudeCode => "claude-code",
            Self::Cursor => "cursor",
            Self::Unknown => "unknown",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "claude" | "claude-code" | "ClaudeCode" => Self::ClaudeCode,
            "cursor" | "Cursor" => Self::Cursor,
            _ => Self::Unknown,
        }
    }
}

impl std::fmt::Display for Harness {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}
