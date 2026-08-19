//! Deterministic frame table: named ranges inside a stored page.
//!
//! This is the page-table walk. `ctx://shell/abc#auth::login` is a virtual
//! address, not a grep.

use ctx_protocol::Frame;

use crate::symbols::collect_symbol_spans;

pub fn extract_frames(kind: &str, payload: &str) -> Vec<Frame> {
    let mut frames = Vec::new();
    match kind {
        "shell" => collect_shell_frames(payload, &mut frames),
        "file" => collect_symbol_frames(payload, &mut frames),
        _ => collect_signal_frames(payload, &mut frames),
    }
    merge_frames(&mut frames);
    frames.truncate(48);
    frames
}

/// A compiler/test mention of a source file, optionally a line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapHit {
    pub path: String,
    pub line: Option<u32>,
}

/// Source paths mentioned in compiler/test output. Prefetch map, not inlined.
pub fn extract_maps(payload: &str) -> Vec<String> {
    let mut paths = Vec::new();
    for hit in extract_map_hits(payload) {
        if !paths.iter().any(|p| p == &hit.path) {
            paths.push(hit.path);
        }
    }
    paths
}

pub fn extract_map_hits(payload: &str) -> Vec<MapHit> {
    let mut out = Vec::new();
    for line in payload.lines() {
        if let Some(hit) = map_hit(line) {
            if !out
                .iter()
                .any(|p: &MapHit| p.path == hit.path && p.line == hit.line)
            {
                out.push(hit);
            }
        }
        if out.len() >= 12 {
            break;
        }
    }
    out
}

fn collect_shell_frames(payload: &str, frames: &mut Vec<Frame>) {
    let lines: Vec<&str> = payload.lines().collect();
    let mut block_name: Option<String> = None;
    let mut block_start = 0u32;
    let mut error_start: Option<(u32, String, String)> = None;
    let mut capture_blocks = true;

    for (i, line) in lines.iter().enumerate() {
        let n = (i + 1) as u32;
        let t = line.trim();

        if t == "successes:" {
            flush_block(frames, &mut block_name, block_start, n.saturating_sub(1));
            capture_blocks = false;
            continue;
        }
        if t == "failures:" {
            flush_block(frames, &mut block_name, block_start, n.saturating_sub(1));
            capture_blocks = true;
            continue;
        }

        if let Some(name) = failed_test_name(t) {
            frames.push(
                Frame::new(&name, "fail", n, n).with_hint(t.chars().take(80).collect::<String>()),
            );
        }

        if capture_blocks {
            if let Some(name) = stdout_block_name(t) {
                flush_block(frames, &mut block_name, block_start, n.saturating_sub(1));
                block_name = Some(name);
                block_start = n;
                continue;
            }
        }

        if t.starts_with("test result:") || t.starts_with("====") {
            flush_block(frames, &mut block_name, block_start, n.saturating_sub(1));
        }

        if let Some(code) = rustc_error_code(t) {
            if let Some((start, name, hint)) = error_start.take() {
                frames.push(Frame::new(name, "error", start, n.saturating_sub(1)).with_hint(hint));
            }
            error_start = Some((n, code, t.chars().take(100).collect()));
            continue;
        }
        if error_start.is_some()
            && (t.starts_with("error[") || t.starts_with("error: ") || t.starts_with("warning:"))
        {
            if let Some((start, name, hint)) = error_start.take() {
                frames.push(Frame::new(name, "error", start, n.saturating_sub(1)).with_hint(hint));
            }
        }
        if let Some((_, _, hint)) = error_start.as_mut() {
            if hint.len() < 40 {
                if let Some(p) = map_path(line) {
                    *hint = p;
                }
            }
        }

        if let Some(name) = pytest_failed_name(t) {
            frames.push(
                Frame::new(name, "fail", n, n).with_hint(t.chars().take(100).collect::<String>()),
            );
        }
        if t.starts_with("FAIL ") && t.len() > 6 {
            let name = t
                .trim_start_matches("FAIL ")
                .split_whitespace()
                .next()
                .unwrap_or(t);
            frames.push(Frame::new(name, "fail", n, n));
        }
        if let Some(rest) = t.strip_prefix("--- FAIL:") {
            let name = rest.split_whitespace().next().unwrap_or("FAIL");
            frames.push(Frame::new(name, "fail", n, n));
        }
    }
    flush_block(frames, &mut block_name, block_start, lines.len() as u32);
    if let Some((start, name, hint)) = error_start {
        frames.push(Frame::new(name, "error", start, lines.len() as u32).with_hint(hint));
    }
}

fn flush_block(frames: &mut Vec<Frame>, name: &mut Option<String>, start: u32, end: u32) {
    if let Some(name) = name.take() {
        if end >= start {
            frames.push(Frame::new(name, "fail", start, end));
        }
    }
}

fn failed_test_name(t: &str) -> Option<String> {
    let rest = t.strip_prefix("test ")?;
    let (name, status) = rest.split_once(" ... ")?;
    if status.contains("FAILED") || status.contains("failed") {
        Some(name.trim().to_string())
    } else {
        None
    }
}

fn stdout_block_name(t: &str) -> Option<String> {
    let t = t.trim();
    if !t.starts_with("---- ") {
        return None;
    }
    let rest = t.trim_start_matches("---- ").trim_end_matches(" ----");
    let name = rest
        .strip_suffix(" stdout")
        .or_else(|| rest.strip_suffix(" stderr"))?;
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

fn rustc_error_code(t: &str) -> Option<String> {
    let rest = t.strip_prefix("error[")?;
    let code = rest.split(']').next()?;
    if code.starts_with('E') {
        Some(code.to_string())
    } else {
        None
    }
}

fn pytest_failed_name(t: &str) -> Option<String> {
    let rest = t.strip_prefix("FAILED ")?;
    let name = rest.split_whitespace().next()?;
    Some(name.to_string())
}

fn collect_symbol_frames(payload: &str, frames: &mut Vec<Frame>) {
    for span in collect_symbol_spans(payload) {
        frames.push(
            Frame::new(span.name, "symbol", span.start_line, span.end_line)
                .with_hint(span.signature),
        );
    }
}

fn collect_signal_frames(payload: &str, frames: &mut Vec<Frame>) {
    for (i, line) in payload.lines().enumerate() {
        let l = line.to_ascii_lowercase();
        if l.contains("error") || l.contains("fail") || l.contains("panic") {
            let name = format!("L{}", i + 1);
            frames.push(
                Frame::new(name, "signal", (i + 1) as u32, (i + 1) as u32)
                    .with_hint(line.chars().take(80).collect::<String>()),
            );
        }
        if frames.len() >= 24 {
            break;
        }
    }
}

fn map_path(line: &str) -> Option<String> {
    map_hit(line).map(|h| h.path)
}

fn map_hit(line: &str) -> Option<MapHit> {
    let t = line.trim();
    if let Some(rest) = t.strip_prefix("--> ") {
        return path_line(rest, ':');
    }
    if let Some(idx) = t.find("panicked at ") {
        let rest = &t[idx + "panicked at ".len()..];
        return path_line(rest, ':');
    }
    if let Some(rest) = t.strip_prefix("File \"") {
        let path = rest.split('"').next()?;
        let after = rest.split('"').nth(1).unwrap_or("");
        let line = after
            .split("line ")
            .nth(1)
            .and_then(|s| s.split(',').next())
            .and_then(|s| s.trim().parse().ok());
        return Some(MapHit {
            path: sane_path(path)?,
            line,
        });
    }
    // tsc: src/foo.ts(12,3): error TS2304
    if t.contains("error TS") {
        if let Some(idx) = t.find('(') {
            let path = t[..idx].trim();
            if let Some(p) = sane_path(path) {
                let line = t[idx + 1..].split(',').next().and_then(|s| s.parse().ok());
                return Some(MapHit { path: p, line });
            }
        }
    }
    // go: foo.go:12: message
    if let Some((path, rest)) = t.split_once(".go:") {
        if let Some(p) = sane_path(&format!("{path}.go")) {
            let line = rest.split(':').next().and_then(|s| s.parse().ok());
            return Some(MapHit { path: p, line });
        }
    }
    // jest / node: at Object (src/foo.ts:12:5)
    if let Some(idx) = t.rfind('(') {
        let inner = t[idx + 1..].trim_end_matches(')');
        if let Some(hit) = path_line(inner, ':') {
            return Some(hit);
        }
    }
    None
}

fn path_line(rest: &str, sep: char) -> Option<MapHit> {
    let path = rest.split(sep).next()?.trim();
    let line = rest
        .split(sep)
        .nth(1)
        .and_then(|s| s.split(sep).next())
        .and_then(|s| s.trim().parse().ok());
    Some(MapHit {
        path: sane_path(path)?,
        line,
    })
}

fn sane_path(path: &str) -> Option<String> {
    if path.len() < 3 || path.len() > 240 {
        return None;
    }
    if path.contains("://") {
        return None;
    }
    let lower = path.to_ascii_lowercase();
    if !(lower.contains('/')
        || lower.ends_with(".rs")
        || lower.ends_with(".py")
        || lower.ends_with(".ts")
        || lower.ends_with(".js")
        || lower.ends_with(".go")
        || lower.ends_with(".tsx")
        || lower.ends_with(".jsx"))
    {
        return None;
    }
    Some(path.replace('\\', "/"))
}

fn merge_frames(frames: &mut Vec<Frame>) {
    frames.sort_by(|a, b| a.name.cmp(&b.name).then(a.start_line.cmp(&b.start_line)));
    let mut out: Vec<Frame> = Vec::new();
    for f in frames.drain(..) {
        if let Some(last) = out.last_mut() {
            let adjacent = f.start_line <= last.end_line.saturating_add(2);
            if last.name == f.name && last.kind == f.kind && adjacent {
                last.end_line = last.end_line.max(f.end_line);
                if last.hint.is_empty() {
                    last.hint = f.hint.clone();
                }
                continue;
            }
        }
        out.push(f);
    }
    *frames = out;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> String {
        let path = format!(
            "{}/../../benchmarks/fixtures/{name}",
            env!("CARGO_MANIFEST_DIR")
        );
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path}: {e}"))
    }

    #[test]
    fn cargo_fail_names_frames() {
        let frames = extract_frames("shell", &fixture("cargo-test-fail.txt"));
        let names: Vec<_> = frames.iter().map(|f| f.name.as_str()).collect();
        assert!(names.contains(&"auth::login"), "{names:?}");
        assert!(names.contains(&"oauth::redirect"), "{names:?}");
        assert!(
            !names.contains(&"foo::bar"),
            "passing stdout must not become a frame: {names:?}"
        );
        let login = frames
            .iter()
            .filter(|f| f.name == "auth::login")
            .max_by_key(|f| f.end_line.saturating_sub(f.start_line))
            .unwrap();
        assert!(login.end_line > login.start_line, "{login:?}");
        assert!(
            login.start_line >= 24 && login.end_line <= 32,
            "login frame must be the stdout block, not the whole page: {login:?}"
        );
    }

    #[test]
    fn maps_panic_paths() {
        let maps = extract_maps(&fixture("cargo-test-fail.txt"));
        assert!(maps.iter().any(|p| p.contains("auth.rs")), "{maps:?}");
    }

    #[test]
    fn rustc_error_is_a_frame() {
        let raw = "error[E0308]: mismatched types\n  --> src/lib.rs:10:5\n   |\n10 |     x\n";
        let frames = extract_frames("shell", raw);
        assert!(frames.iter().any(|f| f.name == "E0308"), "{frames:?}");
        let maps = extract_maps(raw);
        assert_eq!(maps, vec!["src/lib.rs"]);
    }

    #[test]
    fn file_symbol_frame_covers_body() {
        let src = "fn other() { 1 }\npub fn login(x: i32) -> i32 {\n    let y = x + 1;\n    y\n}\n";
        let frames = extract_frames("file", src);
        let login = frames.iter().find(|f| f.name == "login").expect("login");
        assert!(login.end_line > login.start_line, "{login:?}");
        assert!(login.start_line >= 2 && login.end_line <= 5, "{login:?}");
    }

    #[test]
    fn rustc_span_is_a_line_hit() {
        let raw = "error[E0308]: mismatched types\n  --> src/auth.rs:82:5\n   |\n";
        let hits = extract_map_hits(raw);
        assert_eq!(hits.len(), 1, "{hits:?}");
        assert_eq!(hits[0].path, "src/auth.rs");
        assert_eq!(hits[0].line, Some(82));
    }

    #[test]
    fn jest_paren_span_is_a_line_hit() {
        let raw = "    at Object.<anonymous> (src/auth.test.ts:12:5)\n";
        let hits = extract_map_hits(raw);
        assert_eq!(hits[0].path, "src/auth.test.ts");
        assert_eq!(hits[0].line, Some(12));
    }
}
