use ctx_core::{
    catalog_json, fmt_compact, start_of_today, Config, CtxPaths, PriceBook, Runtime, Snapshot,
    Store,
};
use serde_json::{json, Value};

pub fn run(json: bool) -> anyhow::Result<()> {
    if json {
        println!("{}", payload()?);
        return Ok(());
    }
    let rt = Runtime::open_default()?;
    let snap = Snapshot::capture(&rt.store)?;
    println!("{}", render(rt.config.enabled, &snap));
    Ok(())
}

pub fn payload() -> anyhow::Result<Value> {
    let rt = Runtime::open_default()?;
    let snap = Snapshot::capture(&rt.store)?;
    let recent = notable_rows(&rt.store);
    let book = load_price_book(&rt.config);
    let models = priced_model_rows(&rt.store, start_of_today(), &book)?;
    let money = price_summary(&models);
    Ok(json!({
        "ok": true,
        "enabled": rt.config.enabled,
        "version": env!("CARGO_PKG_VERSION"),
        "today": totals_json(snap.today),
        "week": totals_json(snap.week),
        "by_harness": snap.by_harness_today.iter().map(|(name, t)| {
            json!({
                "name": name,
                "raw": t.raw,
                "delivered": t.delivered,
                "avoided": t.avoided,
                "reduction_pct": t.reduction_pct(),
            })
        }).collect::<Vec<_>>(),
        "models": models,
        "avoided_usd": money.avoided_usd,
        "avoided_usd_estimated": money.estimated,
        "priced_avoided": money.priced_avoided,
        "priced_models": money.priced_models,
        "unpriced_models": money.unpriced_models,
        "default_billing_model": rt.config.default_billing_model,
        "composition": composition_rows(&rt.store, start_of_today(), snap.today),
        "reasons": snap.reasons_today,
        "recent": recent,
        "highlight": recent.first().cloned().unwrap_or(json!(null)),
        "pages": snap.pages,
        "store_bytes": snap.store_bytes,
        "tools": crate::harnesses::payload()["harnesses"].clone(),
    }))
}

/// Local dashboard. `range` is today | 1d | 24h | 7d | 14d | 30d | custom.
/// `from`/`to` are unix seconds for a custom window. `model` is `all`, a model
/// id, or `__unknown__` for sessions whose harness did not report a model.
pub fn dashboard(range: &str, model: &str, from: Option<i64>, to: Option<i64>) -> anyhow::Result<Value> {
    let rt = Runtime::open_default()?;
    let now = now_unix();
    let tz = 8 * 3600;
    let (range, since, until, mut bucket) = resolve_window(range, from, to, now, tz);
    let (start_bucket, _last_bucket, count) = {
        let mut first = align(since, bucket, tz);
        let mut last = align(until.max(since + 1), bucket, tz);
        while bucket < 7 * 86400 && ((last - first) / bucket) + 1 > 48 {
            bucket *= 2;
            first = align(since, bucket, tz);
            last = align(until, bucket, tz);
        }
        let n = (((last - first) / bucket) as usize).saturating_add(1).max(1);
        (first, last, n)
    };
    let selected_model = match model {
        "" | "all" => None,
        other => Some(other),
    };
    let totals = rt.store.dashboard_totals(since, selected_model)?;
    let book = load_price_book(&rt.config);
    let mut models = rt
        .store
        .dashboard_models(since)?
        .into_iter()
        .collect::<Vec<_>>();
    models.sort_by(|a, b| {
        let au = a.id.is_empty() || a.id == "__unknown__";
        let bu = b.id.is_empty() || b.id == "__unknown__";
        au.cmp(&bu).then(b.totals.avoided.cmp(&a.totals.avoided))
    });
    let all_models = models
        .into_iter()
        .map(|m| {
            let source_harnesses = m
                .source_harnesses
                .iter()
                .map(|harness| harness_label(harness))
                .collect::<Vec<_>>();
            with_price(
                &book,
                json!({
                    "id": m.id,
                    "name": model_label(&m.id),
                    "sessions": m.sessions,
                    "raw": m.totals.raw,
                    "delivered": m.totals.delivered,
                    "avoided": m.totals.avoided,
                    "refetched": m.totals.refetched,
                    "net_avoided": m.totals.net_avoided(),
                    "reduction_pct": m.totals.reduction_pct(),
                    "source_harnesses": source_harnesses,
                }),
            )
        })
        .collect::<Vec<_>>();
    let model_options = all_models
        .iter()
        .map(|m| {
            let name = m["name"].as_str().unwrap_or("");
            let sources = m["source_harnesses"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>();
            let option_name = if m["id"] == "__unknown__" && !sources.is_empty() {
                format!("{name}（{}）", sources.join(" · "))
            } else {
                name.to_string()
            };
            json!({
                "id": m["id"],
                "name": option_name,
                "source_harnesses": m["source_harnesses"],
            })
        })
        .collect::<Vec<_>>();
    let mut models = all_models;
    if let Some(id) = selected_model {
        models.retain(|m| m["id"] == id);
    }
    let money = price_summary(&models);
    let sparse = rt
        .store
        .dashboard_series(since, bucket, tz, selected_model)?;
    let series = (0..count)
        .map(|i| {
            let t = start_bucket + (i as i64) * bucket;
            let hit = sparse.iter().find(|p| p.t == t);
            let (raw, delivered) = hit.map(|p| (p.raw, p.delivered)).unwrap_or((0, 0));
            json!({
                "t": t,
                "label": bucket_label(t, bucket, tz),
                "raw": raw,
                "delivered": delivered,
            })
        })
        .collect::<Vec<_>>();
    let by_harness = rt
        .store
        .dashboard_by_harness(since, selected_model)?
        .into_iter()
        .map(|(id, t)| {
            json!({
                "id": id,
                "name": crate::harnesses::display_name(&id),
                "capability": crate::harnesses::capability(&id),
                "raw": t.raw,
                "delivered": t.delivered,
                "avoided": t.avoided,
                "refetched": t.refetched,
                "net_avoided": t.net_avoided(),
                "reduction_pct": t.reduction_pct(),
            })
        })
        .collect::<Vec<_>>();
    Ok(json!({
        "ok": true,
        "enabled": rt.config.enabled,
        "version": env!("CARGO_PKG_VERSION"),
        "range": range,
        "from": since,
        "to": until,
        "model": selected_model.unwrap_or("all"),
        "date_label": date_label(&series, since, until, tz),
        "totals": totals_json(totals),
        "series": series,
        "models": models,
        "model_options": model_options,
        "avoided_usd": money.avoided_usd,
        "avoided_usd_estimated": money.estimated,
        "priced_avoided": money.priced_avoided,
        "priced_models": money.priced_models,
        "unpriced_models": money.unpriced_models,
        "default_billing_model": rt.config.default_billing_model,
        "composition": composition_rows(&rt.store, since, totals),
        "by_harness": by_harness
            .into_iter()
            .filter(|row| {
                let id = row["id"].as_str().unwrap_or("");
                matches!(id, "cursor" | "cursor-cli" | "claude" | "claude-code" | "codex")
            })
            .collect::<Vec<_>>(),
        "tools": crate::harnesses::payload()["harnesses"].clone(),
        "price_catalog": catalog_json(),
        "pages": rt.store.page_count().unwrap_or(0),
        "store_bytes": rt.store.compressed_bytes().unwrap_or(0),
        "reasons": optimizer_rows(&rt.store, since),
        "recent": feed_rows(&rt.store),
        "snapshots": snapshot_rows(&rt.store),
    }))
}

/// One bar: what actually reached the model, then why the rest did not.
/// Segments sum to raw so the bar is readable as the whole context.
fn composition_rows(store: &Store, since: i64, totals: ctx_core::TokenTotals) -> Vec<Value> {
    let mut rows = vec![json!({
        "key": "delivered",
        "label": "有效输入",
        "tokens": totals.delivered,
        "kept": true,
    })];
    let reasons = store.reason_breakdown_since(since).unwrap_or_default();
    let tallied: u64 = reasons.iter().map(|(_, n)| *n).sum();
    let mut sorted = reasons;
    sorted.sort_by_key(|(_, tokens)| std::cmp::Reverse(*tokens));
    for (label, tokens) in sorted {
        if tokens == 0 {
            continue;
        }
        rows.push(json!({
            "key": label,
            "label": reason_short(&label),
            "tokens": tokens,
            "kept": false,
        }));
    }
    // Reason tallies can lag the token totals; keep the bar honest instead of
    // silently rescaling the named segments.
    if let Some(rest) = totals.avoided.checked_sub(tallied) {
        if rest > 0 && totals.avoided > 0 {
            rows.push(json!({
                "key": "other",
                "label": "其他优化",
                "tokens": rest,
                "kept": false,
            }));
        }
    }
    rows
}

fn optimizer_rows(store: &Store, since: i64) -> Vec<Value> {
    let rows = store.reason_breakdown_since(since).unwrap_or_default();
    let total: u64 = rows.iter().map(|(_, n)| *n).sum::<u64>().max(1);
    rows.into_iter()
        .map(|(label, tokens)| {
            json!({
                "label": reason_short(&label),
                "tokens": tokens,
                "pct": (tokens * 100) / total,
            })
        })
        .collect()
}

fn feed_rows(store: &Store) -> Vec<Value> {
    let Ok(mut rows) = store.observations_since(now_unix().saturating_sub(7 * 86400)) else {
        return vec![];
    };
    rows.sort_by(|a, b| b.created_at.cmp(&a.created_at).then(b.id.cmp(&a.id)));
    rows.truncate(12);
    rows.into_iter()
        .map(|o| {
            let tool = o
                .tool_name
                .as_deref()
                .or(o.tool_type.as_deref())
                .unwrap_or(o.event_type.as_str());
            json!({
                "label": tool,
                "opt": o.optimizer,
                "uri": o.uri,
                "raw": o.raw_tokens,
                "delivered": o.delivered_tokens,
            })
        })
        .collect()
}

fn snapshot_rows(store: &Store) -> Vec<Value> {
    store
        .list_snapshots()
        .unwrap_or_default()
        .into_iter()
        .take(6)
        .map(|s| {
            json!({
                "id": s.id,
                "note": s.note,
                "created_at": s.created_at,
            })
        })
        .collect()
}

fn harness_label(id: &str) -> String {
    match id {
        "cursor" => "Cursor".into(),
        "claude" | "claude-code" => "Claude Code".into(),
        "unknown" | "" => "其他".into(),
        other => other.to_string(),
    }
}

fn model_label(id: &str) -> String {
    match id {
        "__unknown__" | "" => "未上报模型".into(),
        "default" | "auto" => "Auto（自动选择）".into(),
        "gpt-5" => "GPT-5".into(),
        "claude-sonnet-4-6" => "Claude Sonnet 4.6".into(),
        "claude-opus-4-1" => "Claude Opus 4.1".into(),
        "gemini-2.5-pro" => "Gemini 2.5 Pro".into(),
        "deepseek-chat" => "DeepSeek Chat".into(),
        other => other.to_string(),
    }
}

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn align(ts: i64, bucket: i64, tz: i64) -> i64 {
    ((ts + tz) / bucket) * bucket - tz
}

fn bucket_label(ts: i64, bucket: i64, tz: i64) -> String {
    let local = ts + tz;
    let (y, m, d, h) = civil(local);
    if bucket >= 86400 {
        format!("{m}月{d}日")
    } else if bucket >= 3600 {
        format!("{h:02}:00")
    } else {
        format!("{y}-{m:02}-{d:02}")
    }
}

/// Unix seconds → civil date. `ts` should already be shifted to the display zone.
fn civil(ts: i64) -> (i32, u32, u32, u32) {
    let days = ts.div_euclid(86400);
    let secs = ts.rem_euclid(86400) as u32;
    let h = secs / 3600;
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i32 + era as i32 * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = y + i32::from(m <= 2);
    (y, m, d, h)
}

fn notable_rows(store: &Store) -> Vec<Value> {
    let Ok(mut rows) = store.observations_since(start_of_today()) else {
        return vec![];
    };
    rows.retain(|o| o.raw_tokens >= 80);
    rows.sort_by(|a, b| {
        b.raw_tokens
            .cmp(&a.raw_tokens)
            .then(b.avoided_tokens.cmp(&a.avoided_tokens))
    });
    rows.truncate(8);
    rows.into_iter()
        .map(|o| {
            let tool = o
                .tool_name
                .as_deref()
                .filter(|s| !s.is_empty())
                .unwrap_or(o.event_type.as_str());
            let label = match o.optimizer.as_deref() {
                Some("passthrough") | None => tool.to_string(),
                Some(opt) => format!("{tool} · {}", reason_short(opt)),
            };
            json!({
                "label": label,
                "raw": o.raw_tokens,
                "delivered": o.delivered_tokens,
                "avoided": o.avoided_tokens,
            })
        })
        .collect()
}

fn reason_short(key: &str) -> &'static str {
    match key {
        "cow" | "copy-on-write delta" => "写时复制",
        "shell" | "test output noise" => "测试噪音",
        "file-read" | "duplicate file reads" => "重复读文件",
        "duplicate" | "repeated tool output" => "重复输出",
        "mcp" | "mcp json payload" => "MCP JSON",
        "irrelevant log regions" => "无关日志",
        _ => "削减",
    }
}

fn load_price_book(config: &Config) -> PriceBook {
    match CtxPaths::default_home() {
        Ok(paths) => PriceBook::load_with_refresh(&paths, &config.default_billing_model),
        Err(_) => PriceBook::load(
            &CtxPaths::from_root(std::path::PathBuf::from(".")),
            &config.default_billing_model,
        ),
    }
}

fn resolve_window(
    range: &str,
    from: Option<i64>,
    to: Option<i64>,
    now: i64,
    tz: i64,
) -> (&'static str, i64, i64, i64) {
    let custom = from.zip(to).map(|(a, b)| {
        let start = a.min(b);
        let end = a.max(b).clamp(start + 60, now);
        (start, end)
    });
    let (key, since, until) = match (range, custom) {
        ("custom", Some((start, end))) => ("custom", start, end),
        ("today", _) => ("today", start_of_local_day(now, tz), now),
        ("1d" | "24h", _) => ("1d", now.saturating_sub(24 * 3600), now),
        ("14d", _) => ("14d", now.saturating_sub(14 * 86400), now),
        ("30d", _) => ("30d", now.saturating_sub(30 * 86400), now),
        _ => ("7d", now.saturating_sub(7 * 86400), now),
    };
    let span = until.saturating_sub(since).max(1);
    let bucket = if span <= 2 * 86400 {
        3600
    } else if span <= 45 * 86400 {
        86400
    } else {
        7 * 86400
    };
    (key, since, until, bucket)
}

fn start_of_local_day(now: i64, tz: i64) -> i64 {
    let local = now + tz;
    local - local.rem_euclid(86400) - tz
}

fn date_label(series: &[Value], since: i64, until: i64, tz: i64) -> String {
    let (_, m1, d1, h1) = civil(since + tz);
    let (_, m2, d2, h2) = civil(until + tz);
    let hourly = series
        .first()
        .and_then(|p| p["label"].as_str())
        .map(|s| s.contains(':'))
        .unwrap_or(false);
    if hourly {
        if m1 == m2 && d1 == d2 {
            return format!("{m1}月{d1}日 {h1:02}:00 — {h2:02}:00");
        }
        return format!("{m1}月{d1}日 {h1:02}:00 — {m2}月{d2}日 {h2:02}:00");
    }
    if m1 == m2 && d1 == d2 {
        format!("{m1}月{d1}日")
    } else {
        format!("{m1}月{d1}日 — {m2}月{d2}日")
    }
}

fn priced_model_rows(store: &Store, since: i64, book: &PriceBook) -> anyhow::Result<Vec<Value>> {
    let rows = store.dashboard_models(since)?;
    Ok(rows
        .into_iter()
        .map(|m| {
            with_price(
                book,
                json!({
                    "id": m.id,
                    "name": model_label(&m.id),
                    "sessions": m.sessions,
                    "raw": m.totals.raw,
                    "delivered": m.totals.delivered,
                    "avoided": m.totals.avoided,
                    "refetched": m.totals.refetched,
                    "net_avoided": m.totals.net_avoided(),
                    "reduction_pct": m.totals.reduction_pct(),
                    "source_harnesses": m.source_harnesses.iter().map(|h| harness_label(h)).collect::<Vec<_>>(),
                }),
            )
        })
        .collect())
}

fn with_price(book: &PriceBook, mut row: Value) -> Value {
    let id = row["id"].as_str().unwrap_or("").to_string();
    let avoided = row["net_avoided"]
        .as_u64()
        .or_else(|| row["avoided"].as_u64())
        .unwrap_or(0);
    match book.quote(&id) {
        Some(quote) => {
            row["input_usd_per_mtok"] = json!(quote.usd_per_mtok);
            row["avoided_usd"] = json!(ctx_core::round_usd(
                avoided as f64 / 1_000_000.0 * quote.usd_per_mtok
            ));
            row["price_source"] = json!(quote.source.as_str());
            row["price_estimate"] = json!(quote.source.is_estimate());
            row["priced_as"] = json!(quote.matched_id);
        }
        None => {
            row["input_usd_per_mtok"] = Value::Null;
            row["avoided_usd"] = Value::Null;
            row["price_source"] = Value::Null;
            row["price_estimate"] = json!(false);
        }
    }
    row
}

struct PriceSummary {
    avoided_usd: Value,
    priced_avoided: u64,
    priced_models: usize,
    unpriced_models: usize,
    /// Any contributing row priced through the Auto/unknown fallback.
    estimated: bool,
}

fn price_summary(models: &[Value]) -> PriceSummary {
    let mut quoted_usd = 0.0;
    let mut estimated_usd = 0.0;
    let mut priced_avoided = 0u64;
    let mut quoted_models = 0usize;
    let mut estimated_models = 0usize;
    let mut unpriced_models = 0usize;
    for model in models {
        match model["avoided_usd"].as_f64() {
            Some(value) => {
                let estimate = model["price_estimate"].as_bool().unwrap_or(false);
                if estimate {
                    estimated_usd += value;
                    estimated_models += 1;
                } else {
                    quoted_usd += value;
                    quoted_models += 1;
                    priced_avoided += model["net_avoided"]
                        .as_u64()
                        .or_else(|| model["avoided"].as_u64())
                        .unwrap_or(0);
                }
            }
            None => unpriced_models += 1,
        }
    }
    let (avoided_usd, estimated) = if quoted_models > 0 {
        (json!(ctx_core::round_usd(quoted_usd)), false)
    } else if estimated_models > 0 {
        (json!(ctx_core::round_usd(estimated_usd)), true)
    } else {
        (Value::Null, false)
    };
    PriceSummary {
        avoided_usd,
        priced_avoided,
        priced_models: quoted_models,
        unpriced_models,
        estimated,
    }
}

fn totals_json(t: ctx_core::TokenTotals) -> Value {
    json!({
        "raw": t.raw,
        "delivered": t.delivered,
        "avoided": t.avoided,
        "refetched": t.refetched,
        "net_avoided": t.net_avoided(),
        "reduction_pct": t.reduction_pct(),
    })
}

pub fn set_enabled(enabled: bool) -> anyhow::Result<()> {
    let paths = CtxPaths::default_home()?;
    let mut cfg = Config::load(&paths);
    cfg.enabled = enabled;
    cfg.save(&paths)?;
    if enabled {
        println!("CTX resumed. Tool output is virtualized again.");
    } else {
        println!("CTX paused. Tool output passes through. ctx resume to continue.");
    }
    Ok(())
}

pub fn render(enabled: bool, snap: &Snapshot) -> String {
    let enabled = if enabled { "running" } else { "paused" };
    let mut lines = vec![
        format!("CTX               {enabled}"),
        String::new(),
        "Today".into(),
        String::new(),
        format!("Raw context       {:>8}", fmt_compact(snap.today.raw)),
        format!("Delivered         {:>8}", fmt_compact(snap.today.delivered)),
        format!("Avoided           {:>8}", fmt_compact(snap.today.avoided)),
        String::new(),
        format!("Reduction         ↓{}%", snap.today.reduction_pct()),
    ];
    if !snap.by_harness_today.is_empty() {
        lines.push(String::new());
        for (h, t) in &snap.by_harness_today {
            lines.push(format!("{h:<16}  ↓{}%", t.reduction_pct()));
        }
    }
    lines.push(String::new());
    lines.push(format!("Pages stored      {}", snap.pages));
    lines.push(format!("Store             {}", fmt_bytes(snap.store_bytes)));
    lines.push("No context was lost.".into());
    lines.join("\n")
}

fn fmt_bytes(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1} MB", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1} KB", n as f64 / 1_000.0)
    } else {
        format!("{n} B")
    }
}

pub fn home(version: &str, snap: Option<&Snapshot>, enabled: bool) -> String {
    match snap {
        None => format!(
            "\
CTX  {version}
Virtual memory for AI context.

  ctx init       create ~/.ctx
  ctx app        dashboard — tokens kept out
  ctx setup      Claude / Cursor hooks
  ctx demo       see a page fault
  ctx doctor     wiring check

Same result. Less context."
        ),
        Some(snap) => {
            let state = if enabled { "running" } else { "paused" };
            format!(
                "\
CTX  {version}

  {state} · {} pages · ↓{}% today

  ctx status     today's efficiency
  ctx app        dashboard — tokens kept out
  ctx why        why those tokens stayed local
  ctx inspect    HOT / WARM / COLD + URIs
  ctx search     page-fault retrieval
  ctx doctor     wiring check

No context was lost.",
                snap.pages,
                snap.today.reduction_pct()
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_snap() -> Snapshot {
        Snapshot {
            today: Default::default(),
            week: Default::default(),
            by_harness_today: vec![],
            reasons_today: vec![],
            pages: 0,
            store_bytes: 0,
        }
    }

    #[test]
    fn home_first_run_points_to_init() {
        let out = home("0.1.0", None, true);
        assert!(out.contains("ctx init"));
        assert!(out.contains("ctx app"));
        assert!(out.contains("ctx demo"));
        assert!(out.contains("ctx doctor"));
        assert!(!out.contains("clap"));
    }

    #[test]
    fn json_totals_include_reduction() {
        let t = ctx_core::TokenTotals {
            raw: 200,
            delivered: 50,
            avoided: 150,
            refetched: 0,
        };
        let v = totals_json(t);
        assert_eq!(v["avoided"], 150);
        assert_eq!(v["reduction_pct"], 75);
    }

    #[test]
    fn civil_unix_epoch_and_known_day() {
        assert_eq!(civil(0), (1970, 1, 1, 0));
        // 2026-08-18 00:00:00 UTC
        assert_eq!(civil(1_787_011_200), (2026, 8, 18, 0));
    }

    #[test]
    fn dashboard_model_labels_are_readable_without_guessing() {
        assert_eq!(model_label("gpt-5"), "GPT-5");
        assert_eq!(model_label("claude-sonnet-4-6"), "Claude Sonnet 4.6");
        assert_eq!(model_label("claude-opus-4-1"), "Claude Opus 4.1");
        assert_eq!(model_label("__unknown__"), "未上报模型");
        assert_eq!(model_label("default"), "Auto（自动选择）");
        assert_eq!(model_label("future-model-x"), "future-model-x");
    }

    #[test]
    fn unknown_model_uses_grok_list_price() {
        let book = PriceBook::load(
            &CtxPaths::from_root(std::path::PathBuf::from("/tmp/ctx-no-prices")),
            "",
        );
        let row = with_price(&book, json!({"id": "__unknown__", "avoided": 1_000_000u64}));
        assert_eq!(row["avoided_usd"], 2.0);
        assert_eq!(row["price_estimate"], true);
        let money = price_summary(&[row]);
        assert_eq!(money.avoided_usd, json!(2.0));
        assert!(money.estimated);
        assert_eq!(money.unpriced_models, 0);
    }

    #[test]
    fn quoted_models_do_not_mix_auto_estimates_into_the_kpi() {
        let book = PriceBook::load(
            &CtxPaths::from_root(std::path::PathBuf::from("/tmp/ctx-no-prices")),
            "",
        );
        let named = with_price(
            &book,
            json!({"id": "claude-sonnet-4-6", "avoided": 1_000_000u64}),
        );
        let auto = with_price(&book, json!({"id": "default", "avoided": 1_000_000u64}));
        assert_eq!(named["price_estimate"], false);
        assert_eq!(auto["price_estimate"], true);
        let money = price_summary(&[named, auto]);
        assert_eq!(money.avoided_usd, json!(3.0));
        assert!(!money.estimated);
        assert_eq!(money.priced_models, 1);
    }

    #[test]
    fn priced_row_contributes_api_equivalent_dollars() {
        let book = PriceBook::load(
            &CtxPaths::from_root(std::path::PathBuf::from("/tmp/ctx-no-prices")),
            "",
        );
        let row = with_price(
            &book,
            json!({"id": "claude-sonnet-4-6", "avoided": 1_000_000u64}),
        );
        assert_eq!(row["avoided_usd"], 3.0);
        let money = price_summary(&[row]);
        assert_eq!(money.avoided_usd, json!(3.0));
        assert_eq!(money.priced_models, 1);
    }

    #[test]
    fn home_with_store_is_one_screen() {
        let mut snap = empty_snap();
        snap.pages = 9;
        let out = home("0.1.0", Some(&snap), true);
        assert!(out.contains("9 pages"));
        assert!(out.contains("ctx inspect"));
        assert!(out.contains("ctx search"));
        assert!(out.contains("ctx doctor"));
        assert!(out.contains("No context was lost."));
    }
}
