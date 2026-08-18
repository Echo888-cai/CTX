/// Formats an exact-duplicate observation. Detection lives in the runtime
/// because it needs the content store.
pub struct DuplicateGuard;

impl DuplicateGuard {
    pub fn render(uri: &str, count: i64, kind: &str, tool_name: Option<&str>) -> String {
        let who = match tool_name {
            Some(name) if !name.is_empty() && name != kind => format!("{name} ({kind})"),
            Some(name) if !name.is_empty() => name.to_string(),
            _ => kind.to_string(),
        };
        format!("dup {who} ×{count}  {uri}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stub_names_tool_and_kind() {
        let text = DuplicateGuard::render("ctx://shell/abc", 3, "shell", Some("Bash"));
        assert!(text.contains("Bash (shell)"), "{text}");
        assert!(text.contains("ctx://shell/abc"), "{text}");
        assert!(text.contains("×3"), "{text}");
        assert!(!text.contains("id="), "{text}");
        assert!(!text.contains("No context was lost"), "{text}");
    }
}
