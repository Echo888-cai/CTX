//! Split `claude-opus-4-7-thinking-max` into base + effort + provider.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelId {
    pub raw: String,
    pub base: String,
    pub effort: String,
    pub thinking: bool,
    pub provider: String,
}

pub fn parse_model(raw: &str) -> ModelId {
    let raw = raw.trim();
    let mut rest = strip_prefix(raw);
    let provider = provider_of(raw);
    let mut thinking = rest.contains("thinking") || rest.contains("reason");
    let mut effort = String::new();
    for suf in ["-thinking-max", "-thinking", "-xhigh", "-high", "-medium", "-med", "-low", "-max", "-fast"]
    {
        if let Some(stripped) = rest.strip_suffix(suf) {
            if suf.contains("thinking") {
                thinking = true;
            }
            if !matches!(suf, "-thinking-max" | "-thinking" | "-max") {
                effort = suf.trim_start_matches('-').to_string();
            } else if suf == "-thinking-max" || suf == "-max" {
                effort = "max".into();
            }
            rest = stripped;
            break;
        }
    }
    ModelId {
        raw: raw.to_string(),
        base: rest.to_string(),
        effort,
        thinking,
        provider,
    }
}

fn strip_prefix(id: &str) -> &str {
    for p in ["cursor/", "openai/", "anthropic/", "google/", "xai/"] {
        if let Some(rest) = id.strip_prefix(p) {
            return rest;
        }
    }
    id
}

fn provider_of(id: &str) -> String {
    let l = id.to_ascii_lowercase();
    if l.contains("claude") || l.contains("anthropic") {
        "anthropic".into()
    } else if l.contains("gpt") || l.contains("o3") || l.contains("o4") || l.contains("codex") {
        "openai".into()
    } else if l.contains("grok") {
        "xai".into()
    } else if l.contains("gemini") {
        "google".into()
    } else if l.contains("deepseek") {
        "deepseek".into()
    } else if l.contains("composer") {
        "cursor".into()
    } else {
        String::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_cursor_thinking_slug() {
        let m = parse_model("claude-opus-4-7-thinking-max");
        assert_eq!(m.base, "claude-opus-4-7");
        assert!(m.thinking);
        assert_eq!(m.effort, "max");
        assert_eq!(m.provider, "anthropic");
    }

    #[test]
    fn keeps_clean_base() {
        let m = parse_model("claude-sonnet-4-6");
        assert_eq!(m.base, "claude-sonnet-4-6");
        assert!(!m.thinking);
        assert!(m.effort.is_empty());
    }
}
