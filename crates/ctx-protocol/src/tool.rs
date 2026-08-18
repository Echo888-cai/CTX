use serde::{Deserialize, Serialize};

/// Logical tool family. Core routes optimizers from this, not harness names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolKind {
    Shell,
    File,
    Mcp,
    Search,
    Edit,
    Generic,
}

impl ToolKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Shell => "shell",
            Self::File => "file",
            Self::Mcp => "mcp",
            Self::Search => "search",
            Self::Edit => "edit",
            Self::Generic => "generic",
        }
    }

    pub fn from_tool_name(name: &str) -> Self {
        let lower = name.to_ascii_lowercase();
        if lower.contains("bash")
            || lower.contains("shell")
            || lower.contains("powershell")
            || lower == "bashoutput"
        {
            return Self::Shell;
        }
        if lower == "read" || lower.contains("readfile") || lower == "readfile" {
            return Self::File;
        }
        if lower.starts_with("mcp") || lower.contains("__") || lower.starts_with("mcp:") {
            return Self::Mcp;
        }
        if lower.contains("grep")
            || lower.contains("glob")
            || lower.contains("search")
            || lower.contains("websearch")
        {
            return Self::Search;
        }
        if lower.contains("edit") || lower.contains("write") || lower.contains("apply") {
            return Self::Edit;
        }
        Self::Generic
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolRef {
    #[serde(rename = "type")]
    pub kind: ToolKind,
    pub name: String,
}

impl ToolRef {
    pub fn new(name: impl Into<String>) -> Self {
        let name = name.into();
        let kind = ToolKind::from_tool_name(&name);
        Self { kind, name }
    }
}
