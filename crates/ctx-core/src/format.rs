use ctx_optimizer::{map_path_token, prepend_command_exit};
use ctx_protocol::{CtxUri, Frame};

pub fn render_virtualized(
    summary: &str,
    uri: &CtxUri,
    raw_tokens: u32,
    delivered_tokens: u32,
) -> String {
    render_virtualized_space(summary, uri, raw_tokens, delivered_tokens, &[], &[])
}

/// Compact handle so the model can page-in. Diagnostics live in `summary`.
pub fn render_virtualized_space(
    summary: &str,
    uri: &CtxUri,
    raw_tokens: u32,
    delivered_tokens: u32,
    frames: &[Frame],
    maps: &[String],
) -> String {
    let mut out = format!("{}\n\n", summary.trim());

    let page = uri.page_key();
    if frames.len() == 1 {
        let addr = uri.clone().with_frame(&frames[0].name);
        out.push_str(&format!("{addr}  {raw_tokens}→{delivered_tokens}\n"));
    } else {
        out.push_str(&format!("{page}  {raw_tokens}→{delivered_tokens}\n"));
        if !frames.is_empty() {
            let names: Vec<String> = frames
                .iter()
                .take(10)
                .map(|f| format!("#{}", f.name))
                .collect();
            out.push_str(&names.join(" "));
            out.push('\n');
        }
    }

    let summary_l = summary.to_ascii_lowercase();
    let extra_maps: Vec<&str> = maps
        .iter()
        .map(|m| m.as_str())
        .filter(|m| {
            if m.contains("ctx://") || m.contains('#') {
                return true;
            }
            let p = map_path_token(m);
            !p.is_empty() && !summary_l.contains(&p.to_ascii_lowercase())
        })
        .take(4)
        .collect();
    if !extra_maps.is_empty() {
        out.push_str(&extra_maps.join("\n"));
        out.push('\n');
    }
    out
}

pub fn session_banner() -> &'static str {
    "CTX on. L0/L1 frozen. ctx_fetch · ctx_search · ctx_read"
}

/// Make `ctx exec` output scan as: command, exit, body, handle.
pub fn ensure_exec_header(command: &str, exit: i32, body: &str, uri: Option<&str>) -> String {
    let mut out = prepend_command_exit(
        if command.is_empty() {
            None
        } else {
            Some(command)
        },
        Some(exit as i64),
        body,
    );
    if !out.ends_with('\n') {
        out.push('\n');
    }
    if let Some(uri) = uri {
        if !body.contains(uri) {
            out.push('\n');
            out.push_str(uri);
            out.push('\n');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use ctx_optimizer::estimate_tokens;

    #[test]
    fn exec_header_adds_command_exit_and_handle() {
        let out = ensure_exec_header("cargo test", 1, "boom\n", Some("ctx://shell/ab"));
        assert!(out.contains("$ cargo test"), "{out}");
        assert!(out.contains("exit 1"), "{out}");
        assert!(out.contains("boom"), "{out}");
        assert!(out.contains("ctx://shell/ab"), "{out}");
    }

    #[test]
    fn exec_header_does_not_duplicate() {
        let body = "$ cargo test\nexit 0\n\nok\n";
        let out = ensure_exec_header("cargo test", 0, body, Some("ctx://shell/ab"));
        assert_eq!(out.matches("exit 0").count(), 1, "{out}");
        assert_eq!(out.matches("$ cargo test").count(), 1, "{out}");
        assert!(out.contains("ctx://shell/ab"), "{out}");
    }

    #[test]
    fn virtualized_footer_is_a_short_handle() {
        let uri = CtxUri::parse("ctx://shell/9ba72f3c1a2e").unwrap();
        let out = render_virtualized("1 failed", &uri, 18000, 400);
        assert!(out.contains("ctx://shell/9ba72f3c1a2e"), "{out}");
        assert!(out.contains("18000→400"), "{out}");
        assert!(!out.contains("No context was lost"), "{out}");
        assert!(!out.contains("Page in:"), "{out}");
        let overhead = estimate_tokens(&out).saturating_sub(estimate_tokens("1 failed"));
        assert!(overhead < 40, "envelope must stay tiny, got {overhead}");
    }

    #[test]
    fn single_frame_is_a_copyable_address() {
        let uri = CtxUri::parse("ctx://shell/9ba72f3c1a2e").unwrap();
        let frames = vec![ctx_protocol::Frame::new("auth::login", "fail", 24, 31)];
        let out = render_virtualized_space(
            "1 failed\npanicked at src/auth.rs:82:5",
            &uri,
            18000,
            400,
            &frames,
            &["src/auth.rs".into()],
        );
        assert!(
            out.contains("ctx://shell/9ba72f3c1a2e#auth::login"),
            "{out}"
        );
        assert!(
            !out.contains("Mapped"),
            "path already in summary must not repeat:\n{out}"
        );
        assert!(!out.contains("Address space"), "{out}");
    }

    #[test]
    fn prefetch_map_keeps_file_frame_address() {
        let uri = CtxUri::parse("ctx://shell/9ba72f3c1a2e").unwrap();
        let out = render_virtualized_space(
            "1 failed\npanicked at src/auth.rs:82:5",
            &uri,
            18000,
            400,
            &[],
            &["ctx://file/81bfa4c2d91e#login  src/auth.rs:82#login".into()],
        );
        assert!(out.contains("ctx://file/81bfa4c2d91e#login"), "{out}");
    }
}
