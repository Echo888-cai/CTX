//! Cache economics: compact only when keeping the hot prefix is more expensive.

use ctx_store::LedgerTurn;
use ctx_telemetry::{PriceBook, round_usd};

#[derive(Debug, Clone)]
pub struct CompactAdvice {
    pub keep_cache: bool,
    pub reason: &'static str,
    pub keep_cost: f64,
    pub rotate_cost: f64,
    pub rebuild_usd: f64,
    pub hit_rate: f64,
    pub cache_dead: bool,
}

/// `remaining_turns` is how many more model rounds we expect in this epoch.
pub fn advise(turns: &[LedgerTurn], remaining_turns: u32, book: &PriceBook) -> CompactAdvice {
    let hit_rate = if turns.is_empty() {
        0.0
    } else {
        turns.iter().map(|t| t.cache_hit_rate()).sum::<f64>() / turns.len() as f64
    };
    let last_read = turns.last().map(|t| t.cache_read_tokens).unwrap_or(0);
    let prev_read = turns
        .iter()
        .rev()
        .nth(1)
        .map(|t| t.cache_read_tokens)
        .unwrap_or(last_read);
    let cache_dead = last_read == 0 && prev_read > 0;

    let per_turn = turns.last().and_then(|t| book.turn_usd(t)).unwrap_or(0.0);
    let keep_cost = round_usd(per_turn * remaining_turns as f64);

    let rebuild = turns
        .last()
        .map(|t| {
            let mut cold = t.clone();
            cold.cache_read_tokens = 0;
            cold.cache_write_5m = t.input_tokens.max(t.cache_read_tokens);
            book.turn_usd(&cold).unwrap_or(per_turn * 2.0)
        })
        .unwrap_or(0.0);
    let cheap_after = per_turn * 0.4;
    let rotate_cost = round_usd(rebuild + cheap_after * remaining_turns.saturating_sub(1) as f64);

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let last = turns.last();
    let ttl_idle = last
        .map(|t| {
            if t.ts <= 0 {
                return false;
            }
            let window = if t.cache_write_1h > 0 { 3300 } else { 270 };
            now.saturating_sub(t.ts) > window && t.cache_read_tokens > 0
        })
        .unwrap_or(false);

    let done = |keep_cache, reason, cache_dead| CompactAdvice {
        keep_cache,
        reason,
        keep_cost,
        rotate_cost,
        rebuild_usd: rebuild,
        hit_rate,
        cache_dead,
    };

    if cache_dead {
        return done(false, "cache-miss-window", true);
    }
    if ttl_idle {
        return done(false, "ttl-idle", true);
    }
    if turns.iter().any(|t| t.is_compaction) {
        return done(false, "already-compacted", cache_dead);
    }
    if remaining_turns == 0 {
        return done(true, "session-ending", cache_dead);
    }
    let keep_cache = keep_cost <= rotate_cost;
    done(
        keep_cache,
        if keep_cache {
            "keep-hot-prefix"
        } else {
            "cheaper-new-epoch"
        },
        cache_dead,
    )
}

/// Counterfactual: rebuild USD we did not pay because we kept the hot prefix.
pub fn avoided_compact_usd(turns: &[LedgerTurn], book: &PriceBook) -> Option<f64> {
    if turns.is_empty() {
        return None;
    }
    let a = advise(turns, 3, book);
    if turns.iter().any(|t| t.is_compaction) || !a.keep_cache {
        return Some(0.0);
    }
    Some(round_usd(a.rebuild_usd))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ctx_store::{CtxPaths, LedgerTurn};
    use ctx_telemetry::PriceBook;

    fn book() -> PriceBook {
        let dir = tempfile::tempdir().unwrap();
        PriceBook::load(&CtxPaths::from_root(dir.path().to_path_buf()), "")
    }

    #[test]
    fn cache_miss_is_the_compaction_window() {
        let t1 = LedgerTurn {
            model_base: "claude-sonnet-4".into(),
            input_tokens: 5_000,
            cache_read_tokens: 35_000,
            ..LedgerTurn::default()
        };
        let t2 = LedgerTurn {
            model_base: "claude-sonnet-4".into(),
            input_tokens: 40_000,
            cache_read_tokens: 0,
            ..LedgerTurn::default()
        };
        let a = advise(&[t1, t2], 3, &book());
        assert!(!a.keep_cache);
        assert_eq!(a.reason, "cache-miss-window");
    }

    #[test]
    fn hot_cache_is_kept() {
        let t = LedgerTurn {
            model_base: "claude-sonnet-4".into(),
            input_tokens: 4_000,
            cache_read_tokens: 36_000,
            output_tokens: 200,
            ..LedgerTurn::default()
        };
        let a = advise(&[t.clone(), t.clone()], 3, &book());
        assert!(a.keep_cache, "{a:?}");
        assert!(avoided_compact_usd(&[t.clone(), t], &book()).unwrap_or(0.0) > 0.0);
    }
}
