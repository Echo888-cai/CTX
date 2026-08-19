//! Lock / fill / spill token budget.
//!
//! Lock is never dropped (exit, fail names, first assertion). Fill is ranked
//! remainder. Spill is `ctx://` + frames — the runtime envelope, not this
//! module. Objective: keep diagnostics, not the shortest string.
//!
//! Caps are differentiated by tool kind, exit status, signal density, and
//! whether the page was previously fetched (historically useful).

use crate::tokens::estimate_tokens;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BudgetStrategy {
    Extreme,
    #[default]
    Balanced,
    Conservative,
}

impl BudgetStrategy {
    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "extreme" | "aggressive" | "min" => Self::Extreme,
            "conservative" | "safe" | "max" => Self::Conservative,
            _ => Self::Balanced,
        }
    }
}

#[derive(Debug, Clone)]
pub struct BudgetHint {
    pub kind: String,
    pub exit_code: Option<i64>,
    pub signal_lines: u32,
    pub fetched_before: bool,
    pub strategy: BudgetStrategy,
}

impl Default for BudgetHint {
    fn default() -> Self {
        Self {
            kind: String::new(),
            exit_code: None,
            signal_lines: 0,
            fetched_before: false,
            strategy: BudgetStrategy::Balanced,
        }
    }
}

pub fn from_parts(kind: &str, metadata: &serde_json::Value, payload: &str) -> BudgetHint {
    let exit_code = json_i64(metadata.get("exit_code"));
    let fetched_before = metadata
        .get("fetched_before")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let strategy = metadata
        .get("budget_strategy")
        .and_then(|v| v.as_str())
        .map(BudgetStrategy::parse)
        .unwrap_or_default();
    BudgetHint {
        kind: kind.to_string(),
        exit_code,
        signal_lines: count_signal_lines(payload),
        fetched_before,
        strategy,
    }
}

pub fn cap_hint(kind: &str, metadata: &serde_json::Value, payload: &str, raw_tokens: u32) -> u32 {
    cap_for(raw_tokens, &from_parts(kind, metadata, payload))
}

/// Working-set cap for a reducer. Floor so a fail still has room; ceiling
/// so a 200k log cannot spend 30k tokens of “helpful” fill.
pub fn cap(raw_tokens: u32) -> u32 {
    cap_for(raw_tokens, &BudgetHint::default())
}

pub fn cap_for(raw_tokens: u32, hint: &BudgetHint) -> u32 {
    let fifteenth = ((raw_tokens as u64 * 15) / 100) as u32;
    let mut cap = fifteenth.clamp(180, 512);
    cap = ((cap as f64) * tool_weight(hint)).round() as u32;
    if hint.signal_lines > 0 {
        cap = cap.saturating_add(hint.signal_lines.min(12).saturating_mul(10));
    }
    if hint.fetched_before {
        cap = cap.saturating_add(64);
    }
    cap = match hint.strategy {
        BudgetStrategy::Extreme => ((cap as f64) * 0.72).round() as u32,
        BudgetStrategy::Balanced => cap,
        BudgetStrategy::Conservative => ((cap as f64) * 1.35).round() as u32,
    };
    let (lo, hi) = bounds(hint);
    cap.clamp(lo, hi)
}

fn tool_weight(hint: &BudgetHint) -> f64 {
    match hint.kind.as_str() {
        "shell" => match hint.exit_code {
            Some(0) => 0.62,
            Some(_) => 1.35,
            None => 1.0,
        },
        "mcp" => {
            if hint.signal_lines > 0 {
                1.3
            } else {
                0.7
            }
        }
        "file" => 1.0,
        _ => 1.0,
    }
}

fn bounds(hint: &BudgetHint) -> (u32, u32) {
    match (hint.kind.as_str(), hint.exit_code) {
        ("shell", Some(0)) => (120, 280),
        ("shell", Some(_)) => (220, 720),
        ("mcp", _) if hint.signal_lines == 0 => (120, 320),
        _ => (160, 640),
    }
}

pub fn count_signal_lines(text: &str) -> u32 {
    text.lines()
        .filter(|l| {
            let s = l.to_ascii_lowercase();
            s.contains("fail")
                || s.contains("error")
                || s.contains("panic")
                || s.contains("exception")
        })
        .count()
        .min(10_000) as u32
}

fn json_i64(v: Option<&serde_json::Value>) -> Option<i64> {
    let v = v?;
    v.as_i64()
        .or_else(|| v.as_u64().and_then(|n| i64::try_from(n).ok()))
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

    #[test]
    fn failing_shell_gets_more_than_passing_shell() {
        let fail = cap_for(
            2_000,
            &BudgetHint {
                kind: "shell".into(),
                exit_code: Some(1),
                signal_lines: 8,
                fetched_before: false,
                strategy: BudgetStrategy::Balanced,
            },
        );
        let pass = cap_for(
            2_000,
            &BudgetHint {
                kind: "shell".into(),
                exit_code: Some(0),
                signal_lines: 0,
                fetched_before: false,
                strategy: BudgetStrategy::Balanced,
            },
        );
        assert!(fail > pass, "fail={fail} pass={pass}");
        assert!(fail >= 220, "{fail}");
        assert!(pass <= 280, "{pass}");
    }

    #[test]
    fn fetched_pages_keep_more() {
        let base = cap_for(
            2_000,
            &BudgetHint {
                kind: "shell".into(),
                exit_code: Some(1),
                signal_lines: 2,
                fetched_before: false,
                strategy: BudgetStrategy::Balanced,
            },
        );
        let fetched = cap_for(
            2_000,
            &BudgetHint {
                kind: "shell".into(),
                exit_code: Some(1),
                signal_lines: 2,
                fetched_before: true,
                strategy: BudgetStrategy::Balanced,
            },
        );
        assert!(fetched > base, "fetched={fetched} base={base}");
    }
}
