use ctx_core::{fmt_compact, start_of_today, Config, CtxPaths, Runtime, Snapshot, Store};
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
        "reasons": snap.reasons_today,
        "recent": recent,
        "highlight": recent.first().cloned().unwrap_or(json!(null)),
        "pages": snap.pages,
        "store_bytes": snap.store_bytes,
    }))
}

/// Local dashboard. `range` is 24h | 7d | 30d. `model` is `all`, a model id,
/// or `__unknown__` for sessions whose harness did not report a model.
pub fn dashboard(range: &str, model: &str) -> anyhow::Result<Value> {
    let rt = Runtime::open_default()?;
    let now = now_unix();
    let tz = 8 * 3600;
    let (span, bucket, count) = match range {
        "24h" => (24 * 3600, 3600, 24usize),
        "30d" => (30 * 86400, 86400, 30usize),
        _ => (7 * 86400, 86400, 7usize),
    };
    let since = now.saturating_sub(span);
    let selected_model = match model {
        "" | "all" => None,
        other => Some(other),
    };
    let totals = rt.store.dashboard_totals(since, selected_model)?;
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
            json!({
                "id": m.id,
                "name": model_label(&m.id),
                "sessions": m.sessions,
                "raw": m.totals.raw,
                "delivered": m.totals.delivered,
                "avoided": m.totals.avoided,
                "reduction_pct": m.totals.reduction_pct(),
                "source_harnesses": source_harnesses,
            })
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
    let sparse = rt
        .store
        .dashboard_series(since, bucket, tz, selected_model)?;
    let last_bucket = align(now, bucket, tz);
    let start_bucket = last_bucket - (count as i64 - 1) * bucket;
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
    let from = series
        .first()
        .and_then(|p| p["label"].as_str())
        .unwrap_or("");
    let to = series
        .last()
        .and_then(|p| p["label"].as_str())
        .unwrap_or("");
    Ok(json!({
        "ok": true,
        "enabled": rt.config.enabled,
        "version": env!("CARGO_PKG_VERSION"),
        "range": if matches!(range, "24h" | "30d") { range } else { "7d" },
        "model": selected_model.unwrap_or("all"),
        "date_label": format!("{from} — {to}"),
        "totals": totals_json(totals),
        "series": series,
        "models": models,
        "model_options": model_options,
        "pages": rt.store.page_count().unwrap_or(0),
        "store_bytes": rt.store.compressed_bytes().unwrap_or(0),
        "reasons": optimizer_rows(&rt.store, since),
        "recent": feed_rows(&rt.store),
        "snapshots": snapshot_rows(&rt.store),
    }))
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
        "__unknown__" | "" => "未识别模型".into(),
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

fn totals_json(t: ctx_core::TokenTotals) -> Value {
    json!({
        "raw": t.raw,
        "delivered": t.delivered,
        "avoided": t.avoided,
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
        assert_eq!(model_label("__unknown__"), "未识别模型");
        assert_eq!(model_label("future-model-x"), "future-model-x");
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
