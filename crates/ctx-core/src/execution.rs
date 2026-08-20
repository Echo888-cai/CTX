//! Data plane: deterministic reduction of logs/tests before the model sees them.

use ctx_optimizer::{compact_block, extract_frames, estimate_tokens};

/// Collapse a huge log into an evidence pack the model can reason over.
pub fn evidence_pack(log: &str, max_tokens: u32) -> String {
    let frames = extract_frames("shell", log);
    let mut out = String::new();
    if !frames.is_empty() {
        out.push_str("EVIDENCE\n");
        for f in frames.iter().take(12) {
            out.push_str(&format!(
                "- {}  L{}–{}  {}\n",
                f.name, f.start_line, f.end_line, f.hint
            ));
        }
    }
    let budget_lines = (max_tokens / 8).max(24) as usize;
    let body = compact_block(log, budget_lines);
    out.push('\n');
    out.push_str(&body);
    if estimate_tokens(&out) > max_tokens {
        let mut clipped = String::new();
        for line in out.lines() {
            if estimate_tokens(&clipped) + estimate_tokens(line) + 1 > max_tokens {
                clipped.push_str("…\n");
                break;
            }
            clipped.push_str(line);
            clipped.push('\n');
        }
        return clipped;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pack_is_smaller_than_raw_fail_log() {
        let mut log = String::from("running 80 tests\n");
        for i in 0..80 {
            log.push_str(&format!("test t{i} ... ok\n"));
        }
        log.push_str("test auth::login ... FAILED\nleft: 401\nright: 200\n");
        let pack = evidence_pack(&log, 400);
        assert!(estimate_tokens(&pack) < estimate_tokens(&log));
        assert!(pack.contains("401"), "{pack}");
    }
}
