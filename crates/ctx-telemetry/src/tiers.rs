//! Layered API prices. Cache read is typically 0.1× input; writes cost more.

use serde::Serialize;

use crate::prices::{round_usd, PriceBook};
use ctx_store::LedgerTurn;

/// USD per million tokens for one model family.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct TierRates {
    pub input: f64,
    pub output: f64,
    pub cache_read: f64,
    pub cache_write_5m: f64,
    pub cache_write_1h: f64,
    pub thinking: f64,
}

impl TierRates {
    pub fn anthropic(input: f64) -> Self {
        Self {
            input,
            output: input * 5.0,
            cache_read: input * 0.1,
            cache_write_5m: input * 1.25,
            cache_write_1h: input * 2.0,
            thinking: input * 5.0,
        }
    }

    pub fn openai(input: f64) -> Self {
        Self {
            input,
            output: input * 8.0,
            cache_read: input * 0.25,
            cache_write_5m: input,
            cache_write_1h: input,
            thinking: input * 8.0,
        }
    }

    pub fn xai(input: f64) -> Self {
        Self {
            input,
            output: input * 3.0,
            cache_read: input * 0.1,
            cache_write_5m: input,
            cache_write_1h: input,
            thinking: input * 3.0,
        }
    }

    pub fn generic(input: f64) -> Self {
        Self {
            input,
            output: input * 4.0,
            cache_read: input * 0.1,
            cache_write_5m: input,
            cache_write_1h: input,
            thinking: input * 4.0,
        }
    }

    /// DeepSeek list shape: output ≈ 2× input, cache read ≈ 2% input, writes at input.
    pub fn deepseek(input: f64) -> Self {
        Self {
            input,
            output: input * 2.0,
            cache_read: input * 0.02,
            cache_write_5m: input,
            cache_write_1h: input,
            thinking: input * 2.0,
        }
    }

    pub fn for_model(model: &str, input: f64) -> Self {
        let l = model.to_ascii_lowercase();
        if l.contains("claude") || l.contains("anthropic") {
            Self::anthropic(input)
        } else if l.contains("gpt")
            || l.contains("o3")
            || l.contains("o4")
            || l.contains("codex")
            || l.contains("sol")
            || l.contains("terra")
            || l.contains("luna")
        {
            Self::openai(input)
        } else if l.contains("grok") || l.contains("composer") {
            Self::xai(input)
        } else if l.contains("deepseek") {
            Self::deepseek(input)
        } else {
            Self::generic(input)
        }
    }

    pub fn turn_usd(&self, turn: &LedgerTurn) -> f64 {
        // Match CC Switch CostCalculator: fresh input + output + cache read + cache creation.
        let uncached = turn.uncached_input().max(0) as f64;
        let mut cost = uncached / 1_000_000.0 * self.input
            + turn.cache_read_tokens.max(0) as f64 / 1_000_000.0 * self.cache_read
            + turn.cache_write_5m.max(0) as f64 / 1_000_000.0 * self.cache_write_5m
            + turn.cache_write_1h.max(0) as f64 / 1_000_000.0 * self.cache_write_1h
            + turn.output_tokens.max(0) as f64 / 1_000_000.0 * self.output;
        // OpenAI/Codex Responses usually fold reasoning into output_tokens already.
        let cache_inclusive = turn.harness == "codex" || turn.provider == "openai";
        if !cache_inclusive {
            cost += turn.reasoning_tokens.max(0) as f64 / 1_000_000.0 * self.thinking;
        }
        round_usd(cost)
    }
}

impl PriceBook {
    pub fn tier(&self, model_id: &str) -> Option<TierRates> {
        let quote = self.quote(model_id)?;
        Some(self.overlay_rates(
            &quote.matched_id,
            TierRates::for_model(&quote.matched_id, quote.usd_per_mtok),
        ))
    }

    pub fn turn_usd(&self, turn: &LedgerTurn) -> Option<f64> {
        let id = if !turn.model_base.is_empty() {
            turn.model_base.as_str()
        } else {
            turn.model_raw.as_str()
        };
        Some(self.tier(id)?.turn_usd(turn))
    }

    pub fn turns_usd(&self, turns: &[LedgerTurn]) -> Option<f64> {
        let mut sum = 0.0;
        let mut any = false;
        for turn in turns {
            if let Some(usd) = self.turn_usd(turn) {
                sum += usd;
                any = true;
            }
        }
        any.then_some(round_usd(sum))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ctx_store::CtxPaths;

    #[test]
    fn cache_read_is_tenth_on_claude() {
        let dir = tempfile::tempdir().unwrap();
        let book = PriceBook::load(&CtxPaths::from_root(dir.path().to_path_buf()), "");
        let rates = book.tier("claude-sonnet-4").unwrap();
        assert!((rates.cache_read - rates.input * 0.1).abs() < 1e-9);
        let turn = LedgerTurn {
            model_base: "claude-sonnet-4".into(),
            input_tokens: 10_000,
            cache_read_tokens: 40_000,
            output_tokens: 0,
            ..LedgerTurn::default()
        };
        let usd = book.turn_usd(&turn).unwrap();
        // uncached 10k * 3 + read 40k * 0.3 = 0.03 + 0.012 = 0.042
        assert!(usd > 0.03 && usd < 0.05, "{usd}");
    }

    #[test]
    fn deepseek_tier_uses_two_x_output_not_generic_four() {
        let rates = TierRates::for_model("deepseek-v4-flash", 0.14);
        assert!((rates.output - 0.28).abs() < 1e-9);
        assert!((rates.cache_read - 0.0028).abs() < 1e-9);
        assert!((rates.cache_write_5m - 0.14).abs() < 1e-9);
    }

    #[test]
    fn deepseek_v4_flash_turn_usd() {
        let dir = tempfile::tempdir().unwrap();
        let book = PriceBook::load(&CtxPaths::from_root(dir.path().to_path_buf()), "");
        // 100k uncached @ 0.14 + 500k cache_read @ 0.0028 + 50k write @ 0.14 + 20k out @ 0.28
        // = 0.014 + 0.0014 + 0.007 + 0.0056 = 0.028
        let turn = LedgerTurn {
            model_base: "deepseek-v4-flash".into(),
            input_tokens: 100_000,
            cache_read_tokens: 500_000,
            cache_write_5m: 50_000,
            output_tokens: 20_000,
            ..LedgerTurn::default()
        };
        let usd = book.turn_usd(&turn).unwrap();
        assert!((usd - 0.028).abs() < 1e-6, "{usd}");
    }

    #[test]
    fn deepseek_v4_pro_turn_usd() {
        let dir = tempfile::tempdir().unwrap();
        let book = PriceBook::load(&CtxPaths::from_root(dir.path().to_path_buf()), "");
        // 100k uncached @ 0.435 + 500k cache_read @ 0.003625 + 50k write @ 0.435 + 20k out @ 0.87
        // = 0.0435 + 0.0018125 + 0.02175 + 0.0174 = 0.0844625
        let turn = LedgerTurn {
            model_base: "deepseek-v4-pro".into(),
            input_tokens: 100_000,
            cache_read_tokens: 500_000,
            cache_write_5m: 50_000,
            output_tokens: 20_000,
            ..LedgerTurn::default()
        };
        let usd = book.turn_usd(&turn).unwrap();
        assert!((usd - 0.0844625).abs() < 1e-6, "{usd}");
        // Pro must not price at $0 (missing catalog) or generic flash-like undercount.
        assert!(usd > 0.05, "{usd}");
    }
}
