/// Formats an exact-duplicate observation. Detection lives in the runtime
/// because it needs the content store.
pub struct DuplicateGuard;

impl DuplicateGuard {
    pub fn render(uri: &str, count: i64, kind: &str, tool_name: Option<&str>) -> String {
        Self::render_near(uri, count, kind, tool_name, None, None)
    }

    pub fn render_near(
        uri: &str,
        count: i64,
        kind: &str,
        tool_name: Option<&str>,
        hamming: Option<u32>,
        delta: Option<&str>,
    ) -> String {
        let who = match tool_name {
            Some(name) if !name.is_empty() && name != kind => format!("{name} ({kind})"),
            Some(name) if !name.is_empty() => name.to_string(),
            _ => kind.to_string(),
        };
        let mut text = match hamming {
            Some(n) => format!("dup {who} ×{count}  {uri}  近似（差异 {n}）"),
            None => format!("dup {who} ×{count}  {uri}"),
        };
        if let Some(delta) = delta.filter(|d| !d.is_empty()) {
            text.push('\n');
            text.push_str(delta);
        }
        text
    }

    /// A few changed lines so a near-dup stub is not a black hole.
    pub fn brief_delta(prev: &str, curr: &str, max_lines: usize) -> String {
        let mut out = String::new();
        let mut shown = 0usize;
        for (a, b) in prev.lines().zip(curr.lines()) {
            if a == b {
                continue;
            }
            if shown >= max_lines {
                out.push_str("…\n");
                break;
            }
            out.push_str("- ");
            out.push_str(a);
            out.push('\n');
            out.push_str("+ ");
            out.push_str(b);
            out.push('\n');
            shown += 1;
        }
        if shown == 0 {
            let extra = curr.lines().count().saturating_sub(prev.lines().count());
            if extra > 0 {
                return format!("+{extra} lines");
            }
        }
        out
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

    #[test]
    fn near_stub_includes_delta() {
        let text = DuplicateGuard::render_near(
            "ctx://shell/abc",
            2,
            "shell",
            Some("Bash"),
            Some(1),
            Some("- left: 401\n+ left: 402\n"),
        );
        assert!(text.contains("近似（差异 1）"), "{text}");
        assert!(text.contains("401"), "{text}");
        assert!(text.contains("402"), "{text}");
    }
}
