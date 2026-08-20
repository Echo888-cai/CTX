use serde::{Deserialize, Serialize};

/// Which AI harness produced an event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Harness {
    #[serde(alias = "claude")]
    ClaudeCode,
    Cursor,
    Windsurf,
    Continue,
    JetBrains,
    Aider,
    Codex,
    Copilot,
    #[serde(other)]
    Unknown,
}

impl Harness {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ClaudeCode => "claude-code",
            Self::Cursor => "cursor",
            Self::Windsurf => "windsurf",
            Self::Continue => "continue",
            Self::JetBrains => "jetbrains",
            Self::Aider => "aider",
            Self::Codex => "codex",
            Self::Copilot => "copilot",
            Self::Unknown => "unknown",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "claude" | "claude-code" | "ClaudeCode" => Self::ClaudeCode,
            "cursor" | "Cursor" => Self::Cursor,
            "windsurf" | "Windsurf" | "codeium" => Self::Windsurf,
            "continue" | "Continue" => Self::Continue,
            "jetbrains" | "JetBrains" | "idea" | "goland" | "pycharm" => Self::JetBrains,
            "aider" | "Aider" => Self::Aider,
            "codex" | "openai-codex" | "Codex" | "chatgpt" | "ChatGPT" => Self::Codex,
            "copilot" | "github-copilot" | "Copilot" => Self::Copilot,
            _ => Self::Unknown,
        }
    }
}

impl std::fmt::Display for Harness {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_known_harnesses() {
        assert_eq!(Harness::parse("cursor"), Harness::Cursor);
        assert_eq!(Harness::parse("windsurf"), Harness::Windsurf);
        assert_eq!(Harness::parse("codeium"), Harness::Windsurf);
        assert_eq!(Harness::parse("claude-code"), Harness::ClaudeCode);
        assert_eq!(Harness::parse("continue"), Harness::Continue);
        assert_eq!(Harness::parse("jetbrains"), Harness::JetBrains);
        assert_eq!(Harness::parse("aider"), Harness::Aider);
        assert_eq!(Harness::parse("codex"), Harness::Codex);
        assert_eq!(Harness::parse("copilot"), Harness::Copilot);
        assert_eq!(Harness::parse("nope"), Harness::Unknown);
        assert_eq!(Harness::Windsurf.as_str(), "windsurf");
    }
}
