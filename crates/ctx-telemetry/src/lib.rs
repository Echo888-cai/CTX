//! Honest metrics.
//!
//! We report context avoided and estimated reduction. Dollars use Cursor's
//! public input list price (bundled catalog, official models-and-pricing
//! refresh, prices.json, or default_billing_model).

mod prices;
mod tiers;

use ctx_pager::{start_of_today, start_of_week};
use ctx_store::{Store, TokenTotals};
use serde::Serialize;

pub use prices::{
    catalog_json, is_auto_id, official_price_meta, refresh_official_prices,
    refresh_official_prices_now, round_usd, CatalogEntry, PriceBook, PriceQuote, PriceSource,
};
pub use tiers::TierRates;

#[derive(Debug, Clone, Serialize)]
pub struct Snapshot {
    pub today: TokenTotals,
    pub week: TokenTotals,
    pub by_harness_today: Vec<(String, TokenTotals)>,
    pub reasons_today: Vec<(String, u64)>,
    pub pages: u64,
    pub store_bytes: u64,
}

impl Snapshot {
    pub fn capture(store: &Store) -> ctx_store::Result<Self> {
        Ok(Self {
            today: store.totals_since(start_of_today())?,
            week: store.totals_since(start_of_week())?,
            by_harness_today: store.totals_by_harness_since(start_of_today())?,
            reasons_today: store.reason_breakdown_since(start_of_today())?,
            pages: store.page_count()?,
            store_bytes: store.compressed_bytes()?,
        })
    }
}

pub fn session_report(raw: u64, delivered: u64, pages_available: u64) -> String {
    let avoided = raw.saturating_sub(delivered);
    let pct = if raw == 0 {
        0
    } else {
        ((avoided as f64 / raw as f64) * 100.0).round() as u32
    };
    format!(
        "\
CTX session

  raw        {:>8}
  delivered  {:>8}
  kept out   {:>8}  ({}%)

  No context was lost.
  {} pages available.",
        fmt_compact(raw),
        fmt_compact(delivered),
        fmt_compact(avoided),
        pct,
        pages_available
    )
}

pub fn format_why(reasons: &[(String, u64)], total_avoided: u64, pages: u64) -> String {
    let mut lines = vec![format!(
        "Why did CTX keep {} tokens out of the model?\n",
        fmt_num(total_avoided)
    )];
    let mut sorted = reasons.to_vec();
    sorted.sort_by_key(|b| std::cmp::Reverse(b.1));
    if sorted.is_empty() {
        lines.push("No reductions recorded yet today. Try: ctx demo".into());
    } else {
        for (label, tokens) in sorted {
            if tokens == 0 {
                continue;
            }
            lines.push(format!("{:>8}   {label}", fmt_num(tokens)));
        }
        lines.push(String::new());
        lines.push(
            "CTX is not a black-box compressor. Raw context is still in the local store.".into(),
        );
    }
    lines.push(String::new());
    lines.push(format!("No context was lost. {pages} pages available."));
    lines.join("\n")
}

pub fn fmt_num(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out.chars().rev().collect()
}

pub fn fmt_compact(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.2}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.2}K", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn why_empty_still_has_trust_line() {
        let out = format_why(&[], 0, 0);
        assert!(out.contains("No reductions recorded yet today. Try: ctx demo"));
        assert!(out.contains("No context was lost. 0 pages available."));
    }

    #[test]
    fn why_includes_trust_line() {
        let out = format_why(&[("test output noise".into(), 800)], 800, 12);
        assert!(out.contains("800"));
        assert!(out.contains("test output noise"));
        assert!(out.contains("No context was lost. 12 pages available."));
    }

    #[test]
    fn session_report_is_readable() {
        let out = session_report(12_000, 400, 7);
        assert!(out.contains("CTX session"), "{out}");
        assert!(out.contains("No context was lost."), "{out}");
        assert!(out.contains("7 pages available."), "{out}");
        assert!(!out.contains("╭"), "{out}");
    }
}
