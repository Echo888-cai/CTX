//! Lock / fill / spill token budget.
//!
//! Lock is never dropped (exit, fail names, first assertion). Fill is ranked
//! remainder. Spill is `ctx://` + frames — the runtime envelope, not this
//! module. Objective: keep diagnostics, not the shortest string.

use crate::tokens::estimate_tokens;

/// Working-set cap for a reducer. Floor so a fail still has room; ceiling
/// so a 200k log cannot spend 30k tokens of “helpful” fill.
pub fn cap(raw_tokens: u32) -> u32 {
    let fifteenth = ((raw_tokens as u64 * 15) / 100) as u32;
    fifteenth.clamp(180, 512)
}

/// `lock` is always kept. `fill` lines are appended while under `max_tokens`.
pub fn lock_fill(lock: &str, fill: &[String], max_tokens: u32) -> String {
    let mut out = lock.trim_end().to_string();
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    let mut used = estimate_tokens(&out);
    if used >= max_tokens {
        return out;
    }
    for line in fill {
        let t = line.trim_end();
        if t.is_empty() {
            continue;
        }
        let cost = estimate_tokens(t).saturating_add(1);
        if used.saturating_add(cost) > max_tokens {
            break;
        }
        out.push_str(t);
        out.push('\n');
        used = used.saturating_add(cost);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cap_clamps() {
        assert_eq!(cap(100), 180);
        assert_eq!(cap(4_000), 512);
        assert_eq!(cap(2_000), 300);
    }

    #[test]
    fn lock_survives_tiny_budget() {
        let out = lock_fill("FAIL auth::login\nleft: 401", &["padding".repeat(40)], 40);
        assert!(out.contains("401"), "{out}");
        assert!(out.contains("auth::login"), "{out}");
    }
}
