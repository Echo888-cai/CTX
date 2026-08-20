use ctx_core::{compact_advise, start_of_today, PriceBook, Runtime};
use ctx_ledger::sync_all;

pub fn run(json: bool, sync: bool) -> anyhow::Result<()> {
    let rt = Runtime::open_default()?;
    let report = if sync {
        sync_all(&rt.store)
    } else {
        ctx_ledger::SyncReport::default()
    };
    if sync && !json {
        println!(
            "ledger sync  inserted {}  skipped {}  files {}{}",
            report.inserted,
            report.skipped,
            report.files,
            if report.errors.is_empty() {
                String::new()
            } else {
                format!("  errors {}", report.errors.len())
            }
        );
    }
    let since = start_of_today();
    let totals = rt.store.ledger_totals_since(since)?;
    let turns = rt.store.ledger_since(since)?;
    let book = PriceBook::load(rt.store.paths(), &rt.config.default_billing_model);
    let usd = book.turns_usd(&turns);
    let advice = compact_advise(&turns, 3, &book);
    if json {
        println!(
            "{}",
            serde_json::json!({
                "ok": true,
                "sync": { "inserted": report.inserted, "skipped": report.skipped, "files": report.files },
                "today": totals,
                "usd": usd,
                "advice": {
                    "keep_cache": advice.keep_cache,
                    "reason": advice.reason,
                    "hit_rate": advice.hit_rate,
                    "cache_dead": advice.cache_dead,
                }
            })
        );
        return Ok(());
    }
    println!("CTX ledger (today, measured)\n");
    println!("  turns          {:>8}", totals.turns);
    println!("  input          {:>8}", totals.input_tokens);
    println!("  cache read     {:>8}", totals.cache_read_tokens);
    println!("  cache write    {:>8}", totals.cache_write_tokens);
    println!("  output         {:>8}", totals.output_tokens);
    println!("  compact events {:>8}", totals.compact_events);
    println!(
        "  hit rate         {:>6.1}%",
        advice.hit_rate * 100.0
    );
    match usd {
        Some(n) => println!("  effective USD   ${n:.4}  (measured × list tiers)"),
        None => println!("  effective USD   —  (model unpriced)"),
    }
    if let Some(n) = ctx_core::avoided_compact_usd(&turns, &book) {
        println!("  avoided compact ${n:.4}  (counterfactual rebuild)");
    }
    if !totals.plan_type.is_empty() {
        println!(
            "  quota           {:>6.1}%  {}",
            totals.quota_used_pct.unwrap_or(0.0),
            totals.plan_type
        );
    }
    println!(
        "\n  compact advice  {} ({})",
        if advice.keep_cache {
            "KEEP CACHE"
        } else {
            "NEW EPOCH"
        },
        advice.reason
    );
    Ok(())
}
