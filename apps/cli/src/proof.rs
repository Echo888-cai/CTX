//! Shadow A/B: CTX-on vs shadow (deliver raw) from observations + ledger.

use ctx_core::{start_of_week, PriceBook, Runtime};

pub fn run(json: bool) -> anyhow::Result<()> {
    let rt = Runtime::open_default()?;
    let since = start_of_week();
    let live = rt.store.totals_since_shadow(since, false)?;
    let shadow = rt.store.totals_since_shadow(since, true)?;
    let turns = rt.store.ledger_since(since)?;
    let book = PriceBook::load(rt.store.paths(), &rt.config.default_billing_model);
    let measured = book.turns_usd(&turns);
    let shadow_on = rt.config.shadow_mode;
    if json {
        println!(
            "{}",
            serde_json::json!({
                "ok": true,
                "shadow_mode": shadow_on,
                "live": {
                    "raw": live.raw,
                    "delivered": live.delivered,
                    "avoided": live.avoided,
                    "refetched": live.refetched,
                    "net_avoided": live.net_avoided(),
                },
                "shadow": {
                    "raw": shadow.raw,
                    "delivered": shadow.delivered,
                    "avoided": shadow.avoided,
                    "refetched": shadow.refetched,
                    "net_avoided": shadow.net_avoided(),
                },
                "delta_delivered": live.delivered as i64 - shadow.delivered as i64,
                "ledger_usd": measured,
                "ledger_turns": turns.len(),
            })
        );
        return Ok(());
    }
    println!("CTX proof (this week)\n");
    println!("  shadow mode    {}", if shadow_on { "on" } else { "off" });
    println!("                 {:>10}  {:>10}", "live", "shadow");
    println!("  raw            {:>10}  {:>10}", live.raw, shadow.raw);
    println!(
        "  delivered      {:>10}  {:>10}",
        live.delivered, shadow.delivered
    );
    println!(
        "  net avoided    {:>10}  {:>10}  (estimated)",
        live.net_avoided(),
        shadow.net_avoided()
    );
    match measured {
        Some(n) => println!("  ledger USD     ${n:.4}  (measured, all turns)"),
        None => println!("  ledger USD     —  run `ctx ledger --sync`"),
    }
    println!(
        "\n  Same tasks: CTX on vs shadow (deliver raw).\n  Compare ledger USD and delivered, not raw counts."
    );
    Ok(())
}
