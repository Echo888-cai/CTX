use ctx_core::{
    catalog_json, compact_advise, fmt_compact, start_of_today, token_weighted_hit_rate, Config,
    CtxPaths, LedgerTurn, ModelRow, PriceBook, Runtime, Snapshot, Store,
};
use serde_json::{json, Value};
use std::time::Duration;

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
        "composition": composition_rows_filtered(
            &rt.store,
            start_of_today(),
            i64::MAX,
            snap.today,
            None,
            None,
            None,
        ),
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
/// id, or `__unknown__`. `source` is `all` | `cursor` | `claude` | `codex`.
pub fn dashboard(
    range: &str,
    model: &str,
    source: &str,
    from: Option<i64>,
    to: Option<i64>,
) -> anyhow::Result<Value> {
    let rt = Runtime::open_default()?;
    let sync = ctx_ledger::sync_if_due(&rt.store, Duration::from_millis(800));
    let now = now_unix();
    let tz = local_tz_offset(now);
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
    let selected_source = match source {
        "" | "all" => None,
        "cursor" | "claude" | "codex" => Some(source),
        "claude-code" => Some("claude"),
        other => Some(other),
    };
    let source_ids = selected_source.map(|s| vec![s.to_string()]);
    let book = load_price_book(&rt.config);
    // Claude/Codex: list models from ledger transcripts only (same idea as CC Switch
    // usage). Observations add noise like Other / empty model. Cursor still unions
    // hooks + ledger because Cursor models often appear first in observations.
    let mut raw_models = match selected_source {
        Some("claude") | Some("codex") => ledger_model_rows(
            &rt.store,
            since,
            until,
            selected_source,
            &book,
        ),
        _ => {
            let mut rows = rt
                .store
                .dashboard_models_between(since, until, source_ids.as_deref())?
                .into_iter()
                .collect::<Vec<_>>();
            rows.extend(ledger_model_rows(
                &rt.store,
                since,
                until,
                selected_source,
                &book,
            ));
            rows
        }
    };
    raw_models.retain(|m| is_listed_model(&m.id) && (m.totals.raw > 0 || m.totals.avoided > 0));
    raw_models.sort_by(|a, b| {
        let au = a.id.is_empty() || a.id == "__unknown__";
        let bu = b.id.is_empty() || b.id == "__unknown__";
        au.cmp(&bu)
            .then(b.totals.avoided.cmp(&a.totals.avoided))
            .then(b.totals.raw.cmp(&a.totals.raw))
    });
    let selected_model = match model {
        "" | "all" => None,
        other => {
            let want = book.canonical_id(other);
            let ok = raw_models
                .iter()
                .any(|m| book.canonical_id(&m.id) == want || m.id == other);
            if ok {
                Some(other)
            } else {
                None
            }
        }
    };
    let filter_ids = selected_model.map(|sel| {
        let want = book.canonical_id(sel);
        let mut ids: Vec<String> = raw_models
            .iter()
            .filter(|m| book.canonical_id(&m.id) == want || m.id == sel)
            .map(|m| m.id.clone())
            .collect();
        if ids.is_empty() {
            ids.push(sel.to_string());
        }
        ids
    });
    // Claude/Codex hooks often omit model on observations; map blank-model
    // observations onto the selected model via ledger session ids.
    let attributed_sessions = selected_model
        .map(|sel| ledger_sessions_for_model(&rt.store, since, until, selected_source, &book, sel))
        .unwrap_or_default();
    let session_filter = (!attributed_sessions.is_empty()).then_some(attributed_sessions.as_slice());
    let totals = rt.store.dashboard_totals_for(
        since,
        until,
        filter_ids.as_deref(),
        source_ids.as_deref(),
        session_filter,
    )?;
    let all_models = merge_model_rows(&book, raw_models);
    let model_options = all_models
        .iter()
        .map(|m| {
            json!({
                "id": m["id"],
                "name": m["name"],
                "source_harnesses": m["source_harnesses"],
            })
        })
        .collect::<Vec<_>>();
    let mut models = all_models;
    if let Some(id) = selected_model {
        let want = book.canonical_id(id);
        models.retain(|m| {
            m["id"].as_str() == Some(id) || m["id"].as_str() == Some(want.as_str())
        });
        // Ledger rows carry tokens but not CTX avoided; after session
        // attribution, fill avoided from observation totals for pricing KPIs.
        if totals.avoided > 0 {
            for row in &mut models {
                if row["avoided"].as_u64().unwrap_or(0) == 0 {
                    *row = with_price(
                        &book,
                        json!({
                            "id": row["id"],
                            "name": row["name"],
                            "sessions": row["sessions"],
                            "raw": totals.raw,
                            "delivered": totals.delivered,
                            "avoided": totals.avoided,
                            "refetched": totals.refetched,
                            "net_avoided": totals.net_avoided(),
                            "reduction_pct": totals.reduction_pct(),
                            "source_harnesses": row["source_harnesses"],
                        }),
                    );
                }
            }
        }
    }
    let money = price_summary(&models);
    let sparse = rt.store.dashboard_series_for(
        since,
        until,
        bucket,
        tz,
        filter_ids.as_deref(),
        source_ids.as_deref(),
        session_filter,
    )?;
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
    Ok(json!({
        "ok": true,
        "enabled": rt.config.enabled,
        "version": env!("CARGO_PKG_VERSION"),
        "range": range,
        "from": since,
        "to": until,
        "source": selected_source.unwrap_or("all"),
        "model": selected_model
            .map(|id| book.canonical_id(id))
            .unwrap_or_else(|| "all".into()),
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
        "composition": composition_rows_filtered(
            &rt.store,
            since,
            until,
            totals,
            filter_ids.as_deref(),
            source_ids.as_deref(),
            session_filter,
        ),
        "tools": crate::harnesses::payload()["harnesses"].clone(),
        "price_catalog": catalog_json(),
        "pages": rt.store.page_count().unwrap_or(0),
        "store_bytes": rt.store.compressed_bytes().unwrap_or(0),
        "reasons": optimizer_rows_filtered(
            &rt.store,
            since,
            until,
            filter_ids.as_deref(),
            source_ids.as_deref(),
            session_filter,
        ),
        "recent": feed_rows(&rt.store),
        "snapshots": snapshot_rows(&rt.store),
        "ledger": ledger_json(
            &rt.store,
            since,
            until,
            &book,
            &sync,
            selected_source,
            selected_model,
        ),
        "epochs": rt.store.epoch_count().unwrap_or(0),
        "overlays": rt.store.overlay_count().unwrap_or(0),
        "synced_at": now,
        "sync": {
            "inserted": sync.inserted,
            "skipped": sync.skipped,
            "files": sync.files,
        },
    }))
}

fn ledger_json(
    store: &Store,
    since: i64,
    until: i64,
    book: &PriceBook,
    sync: &ctx_ledger::SyncReport,
    source: Option<&str>,
    model: Option<&str>,
) -> Value {
    let mut turns = store.ledger_between(since, until).unwrap_or_default();
    if let Some(src) = source {
        turns.retain(|t| ledger_source_id(&t.harness) == Some(src));
    }
    if let Some(sel) = model {
        let want = book.canonical_id(sel);
        turns.retain(|t| {
            let id = if t.model_base.is_empty() {
                t.model_raw.as_str()
            } else {
                t.model_base.as_str()
            };
            book.canonical_id(id) == want || id == sel
        });
    }
    let usd = book.turns_usd(&turns);
    let advice = compact_advise(&turns, 3, book);
    let avoided_compact = ctx_core::avoided_compact_usd(&turns, book);
    let hit_rate = ledger_hit_rate(&turns);
    let (faults, recalled) = store.refetch_totals_between(since, until).unwrap_or((0, 0));
    let miss_turns = turns.iter().filter(|t| t.cache_read_tokens == 0).count();
    let miss_tokens: i64 = turns
        .iter()
        .map(|t| {
            if t.cache_read_tokens == 0 {
                t.uncached_input()
            } else {
                0
            }
        })
        .sum();
    let passed = store.clean_sessions_since(since).unwrap_or(0);
    let inferred = turns.iter().filter(|t| t.confidence == "inferred").count();
    let measured = turns
        .iter()
        .filter(|t| t.confidence == "measured" || t.confidence == "partial")
        .count();
    let kind = if turns.is_empty() {
        "empty"
    } else if measured == 0 && inferred > 0 {
        "inferred"
    } else if inferred > 0 {
        "mixed"
    } else {
        "measured"
    };
    let cursor_only = !turns.is_empty() && turns.iter().all(|t| t.harness == "cursor");
    let cache_read: i64 = turns.iter().map(|t| t.cache_read_tokens.max(0)).sum();
    let cache_write: i64 = turns.iter().map(|t| t.cache_write_tokens()).sum();
    let fresh_input: i64 = turns.iter().map(|t| t.uncached_input()).sum();
    let output: i64 = turns.iter().map(|t| t.output_tokens.max(0)).sum();
    let real_total: i64 = turns.iter().map(|t| t.real_total_tokens()).sum();
    let sources = ledger_sources_json(&turns, book);
    json!({
        "kind": kind,
        "confidence": if inferred > 0 && measured == 0 { "inferred" } else { "measured" },
        "cursor_inferred": cursor_only && inferred > 0,
        "turns": turns.len(),
        "input_tokens": fresh_input,
        "output_tokens": output,
        "cache_read_tokens": cache_read,
        "cache_write_tokens": cache_write,
        "real_total_tokens": real_total,
        "cache_miss_turns": miss_turns,
        "cache_miss_tokens": miss_tokens,
        "reasoning_tokens": turns.iter().map(|t| t.reasoning_tokens.max(0)).sum::<i64>(),
        "compact_events": turns.iter().filter(|t| t.is_compaction).count(),
        "avoided_compact_usd": avoided_compact,
        "hit_rate": hit_rate,
        "usd": usd,
        "plan_type": turns.iter().rev().find(|t| !t.plan_type.is_empty()).map(|t| t.plan_type.clone()).unwrap_or_default(),
        "quota_used_pct": turns.iter().rev().find_map(|t| t.quota_used_pct),
        "resets_at": turns.iter().rev().find(|t| !t.resets_at.is_empty()).map(|t| t.resets_at.clone()).unwrap_or_default(),
        "page_faults": faults,
        "recalled_tokens": recalled,
        "retries": faults,
        "task_passed": passed,
        "synced_at": now_unix(),
        "sync_inserted": sync.inserted,
        "sources": sources,
        "advice": {
            "keep_cache": advice.keep_cache,
            "reason": advice.reason,
            "cache_dead": advice.cache_dead,
        }
    })
}

fn ledger_source_id(harness: &str) -> Option<&'static str> {
    match harness {
        "cursor" => Some("cursor"),
        "claude" | "claude-code" => Some("claude"),
        "codex" | "chatgpt" | "openai-codex" => Some("codex"),
        _ => None,
    }
}

fn ledger_source_slice<'a>(turns: &'a [LedgerTurn], id: &str) -> Vec<&'a LedgerTurn> {
    turns
        .iter()
        .filter(|t| ledger_source_id(&t.harness) == Some(id))
        .collect()
}

fn ledger_source_kind(slice: &[&LedgerTurn]) -> &'static str {
    if slice.is_empty() {
        return "empty";
    }
    let inferred = slice.iter().filter(|t| t.confidence == "inferred").count();
    let measured = slice
        .iter()
        .filter(|t| t.confidence == "measured" || t.confidence == "partial")
        .count();
    if measured == 0 && inferred > 0 {
        "inferred"
    } else if inferred > 0 {
        "mixed"
    } else {
        "measured"
    }
}

fn ledger_sources_json(turns: &[LedgerTurn], book: &PriceBook) -> Vec<Value> {
    // Claude Code CLI (~/.claude/projects) · Codex CLI (~/.codex/sessions) · Cursor local.
    // Same cache-normalized math as CC Switch UsageHero / CostCalculator.
    const IDS: &[(&str, &str)] = &[
        ("claude", "Claude Code"),
        ("codex", "Codex"),
        ("cursor", "Cursor"),
    ];
    IDS.iter()
        .map(|(id, label)| {
            let refs = ledger_source_slice(turns, id);
            let owned: Vec<LedgerTurn> = refs.iter().map(|t| (*t).clone()).collect();
            let hit_rate = ledger_hit_rate(&owned);
            let cache_read: i64 = owned.iter().map(|t| t.cache_read_tokens.max(0)).sum();
            let cache_write: i64 = owned.iter().map(|t| t.cache_write_tokens()).sum();
            let fresh_input: i64 = owned.iter().map(|t| t.uncached_input()).sum();
            let output: i64 = owned.iter().map(|t| t.output_tokens.max(0)).sum();
            let real_total: i64 = owned.iter().map(|t| t.real_total_tokens()).sum();
            let usd = book.turns_usd(&owned);
            let kind = ledger_source_kind(&refs);
            json!({
                "id": id,
                "label": label,
                "turns": owned.len(),
                "hit_rate": hit_rate,
                "real_total_tokens": real_total,
                "input_tokens": fresh_input,
                "output_tokens": output,
                "cache_read_tokens": cache_read,
                "cache_write_tokens": cache_write,
                "usd": usd,
                "kind": kind,
                "estimated": *id == "cursor",
            })
        })
        .collect()
}

/// One bar: what actually reached the model, then why the rest did not.
/// Segments sum to raw so the bar is readable as the whole context.
fn composition_rows_filtered(
    store: &Store,
    since: i64,
    until: i64,
    totals: ctx_core::TokenTotals,
    models: Option<&[String]>,
    harnesses: Option<&[String]>,
    sessions: Option<&[String]>,
) -> Vec<Value> {
    let mut rows = vec![json!({
        "key": "delivered",
        "label": "有效输入",
        "tokens": totals.delivered,
        "kept": true,
    })];
    let reasons = store
        .reason_breakdown_for(since, until, models, harnesses, sessions)
        .unwrap_or_default();
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

fn optimizer_rows_filtered(
    store: &Store,
    since: i64,
    until: i64,
    models: Option<&[String]>,
    harnesses: Option<&[String]>,
    sessions: Option<&[String]>,
) -> Vec<Value> {
    let rows = store
        .reason_breakdown_for(since, until, models, harnesses, sessions)
        .unwrap_or_default();
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

/// Sessions whose ledger turns used `model` (for attributing blank-model hooks).
fn ledger_sessions_for_model(
    store: &Store,
    since: i64,
    until: i64,
    source: Option<&str>,
    book: &PriceBook,
    model: &str,
) -> Vec<String> {
    let want = book.canonical_id(model);
    let mut turns = store.ledger_between(since, until).unwrap_or_default();
    if let Some(src) = source {
        turns.retain(|t| ledger_source_id(&t.harness) == Some(src));
    }
    turns.retain(|t| {
        let id = if t.model_base.is_empty() {
            t.model_raw.as_str()
        } else {
            t.model_base.as_str()
        };
        book.canonical_id(id) == want || id == model
    });
    let mut sessions: Vec<String> = turns
        .into_iter()
        .map(|t| t.session)
        .filter(|s| !s.is_empty())
        .collect();
    sessions.sort();
    sessions.dedup();
    sessions
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

/// Provider cache-read share across every harness that wrote ledger turns.
/// Token-weighted, not split by model. Codex input already includes cache reads.
/// Denominator includes cache writes (CC Switch cacheable-input).
fn ledger_hit_rate(turns: &[LedgerTurn]) -> Option<f64> {
    token_weighted_hit_rate(turns)
}

/// Models seen in provider transcripts for the window (Claude/Codex/Cursor ledger).
/// Unioned with observation-based rows so the dropdown is not empty when hooks
/// never recorded those tools.
fn ledger_model_rows(
    store: &Store,
    since: i64,
    until: i64,
    source: Option<&str>,
    book: &PriceBook,
) -> Vec<ModelRow> {
    use std::collections::{BTreeMap, BTreeSet};
    let mut turns = store.ledger_between(since, until).unwrap_or_default();
    if let Some(src) = source {
        turns.retain(|t| ledger_source_id(&t.harness) == Some(src));
    }
    struct Acc {
        sessions: BTreeSet<String>,
        harnesses: Vec<String>,
        tokens: u64,
    }
    let mut map: BTreeMap<String, Acc> = BTreeMap::new();
    for turn in turns {
        let raw_id = if !turn.model_base.is_empty() {
            turn.model_base.as_str()
        } else if !turn.model_raw.is_empty() {
            turn.model_raw.as_str()
        } else {
            "__unknown__"
        };
        let id = if raw_id == "__unknown__" {
            "__unknown__".into()
        } else {
            book.canonical_id(raw_id)
        };
        let harness = ledger_source_id(&turn.harness)
            .unwrap_or(turn.harness.as_str())
            .to_string();
        let entry = map.entry(id).or_insert_with(|| Acc {
            sessions: BTreeSet::new(),
            harnesses: Vec::new(),
            tokens: 0,
        });
        if !turn.session.is_empty() {
            entry.sessions.insert(turn.session.clone());
        }
        if !harness.is_empty() && !entry.harnesses.iter().any(|h| h == &harness) {
            entry.harnesses.push(harness);
        }
        entry.tokens = entry
            .tokens
            .saturating_add(turn.real_total_tokens().max(0) as u64);
    }
    map.into_iter()
        .map(|(id, acc)| ModelRow {
            id,
            sessions: acc.sessions.len() as u64,
            totals: ctx_core::TokenTotals {
                raw: acc.tokens,
                delivered: acc.tokens,
                avoided: 0,
                refetched: 0,
            },
            source_harnesses: acc.harnesses,
        })
        .collect()
}

fn harness_label(id: &str) -> String {
    match id {
        "cursor" => "Cursor".into(),
        "claude" | "claude-code" => "Claude Code".into(),
        "codex" | "chatgpt" | "openai-codex" => "ChatGPT".into(),
        "unknown" | "" => String::new(),
        other => other.to_string(),
    }
}

/// Drop Claude Code internals and unlabeled rows from the model dropdown.
fn is_listed_model(id: &str) -> bool {
    let id = id.trim();
    if id.is_empty() || id == "__unknown__" || id.eq_ignore_ascii_case("unknown") {
        return false;
    }
    if id.eq_ignore_ascii_case("<synthetic>") || id.eq_ignore_ascii_case("synthetic") {
        return false;
    }
    if id.starts_with('<') && id.ends_with('>') {
        return false;
    }
    true
}

/// Real model id, title-cased on `-` segments: `deepseek-v4-pro` → `Deepseek-V4-Pro`.
fn model_label(id: &str) -> String {
    let id = id.trim();
    if id.is_empty() || id == "__unknown__" || id.eq_ignore_ascii_case("unknown") {
        return "Other".into();
    }
    if id.eq_ignore_ascii_case("default") || id.eq_ignore_ascii_case("auto") {
        return "Auto".into();
    }
    id.split('-')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => {
                    let mut out = first.to_uppercase().collect::<String>();
                    out.push_str(chars.as_str());
                    out
                }
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join("-")
}

fn merge_model_rows(book: &PriceBook, rows: Vec<ModelRow>) -> Vec<Value> {
    use std::collections::BTreeMap;
    let mut groups: BTreeMap<String, ModelRow> = BTreeMap::new();
    for row in rows {
        let key = book.canonical_id(&row.id);
        match groups.get_mut(&key) {
            Some(acc) => {
                acc.sessions = acc.sessions.saturating_add(row.sessions);
                acc.totals.raw = acc.totals.raw.saturating_add(row.totals.raw);
                acc.totals.delivered = acc.totals.delivered.saturating_add(row.totals.delivered);
                acc.totals.avoided = acc.totals.avoided.saturating_add(row.totals.avoided);
                acc.totals.refetched = acc.totals.refetched.saturating_add(row.totals.refetched);
                for harness in row.source_harnesses {
                    if !acc.source_harnesses.iter().any(|h| h == &harness) {
                        acc.source_harnesses.push(harness);
                    }
                }
            }
            None => {
                groups.insert(
                    key.clone(),
                    ModelRow {
                        id: key,
                        sessions: row.sessions,
                        totals: row.totals,
                        source_harnesses: row.source_harnesses,
                    },
                );
            }
        }
    }
    let mut merged: Vec<_> = groups.into_values().collect();
    merged.sort_by(|a, b| {
        let au = a.id == "__unknown__" || a.id == "default";
        let bu = b.id == "__unknown__" || b.id == "default";
        au.cmp(&bu).then(b.totals.avoided.cmp(&a.totals.avoided))
    });
    merged
        .into_iter()
        .map(|m| {
            let source_harnesses = m
                .source_harnesses
                .iter()
                .map(|harness| harness_label(harness))
                .filter(|label| !label.is_empty())
                .collect::<Vec<_>>();
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
                    "source_harnesses": source_harnesses,
                }),
            )
        })
        .collect()
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
    _tz: i64,
) -> (&'static str, i64, i64, i64) {
    let custom = from.zip(to).map(|(a, b)| {
        let start = a.min(b).min(now);
        let end = a.max(b).clamp(start, now).max(start);
        (start, end.max(start))
    });
    let (key, since, until) = match (range, custom) {
        ("custom", Some((start, end))) => ("custom", start, end),
        ("today", _) => ("today", start_of_today(), now),
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

fn local_tz_offset(now: i64) -> i64 {
    let elapsed = now.saturating_sub(start_of_today());
    elapsed - now.rem_euclid(86400)
}

#[allow(dead_code)]
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
            if let Some(tier) = book.tier(&id) {
                row["cache_read_usd_per_mtok"] = json!(tier.cache_read);
                row["output_usd_per_mtok"] = json!(tier.output);
            }
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
        assert_eq!(model_label("gpt-5"), "Gpt-5");
        assert_eq!(model_label("deepseek-v4-pro"), "Deepseek-V4-Pro");
        assert_eq!(model_label("deepseek-v4-flash"), "Deepseek-V4-Flash");
        assert_eq!(model_label("claude-sonnet-4-6"), "Claude-Sonnet-4-6");
        assert_eq!(model_label("__unknown__"), "Other");
        assert_eq!(model_label("default"), "Auto");
        assert_eq!(model_label("future-model-x"), "Future-Model-X");
        assert!(!is_listed_model("<synthetic>"));
        assert!(!is_listed_model("__unknown__"));
    }

    #[test]
    fn cache_hit_rate_weights_tokens_across_harnesses() {
        let cursor = LedgerTurn {
            harness: "cursor".into(),
            input_tokens: 1_000,
            cache_read_tokens: 9_000,
            ..LedgerTurn::default()
        };
        let codex = LedgerTurn {
            harness: "codex".into(),
            input_tokens: 10_000,
            cache_read_tokens: 8_000,
            ..LedgerTurn::default()
        };
        let rate = ledger_hit_rate(&[cursor, codex]).unwrap();
        assert!((rate - 0.85).abs() < 1e-9);
        assert!(ledger_hit_rate(&[]).is_none());
        let with_write = LedgerTurn {
            harness: "claude-code".into(),
            input_tokens: 5_000,
            cache_read_tokens: 40_000,
            cache_write_5m: 5_000,
            ..LedgerTurn::default()
        };
        let write_rate = ledger_hit_rate(&[with_write]).unwrap();
        assert!((write_rate - 0.8).abs() < 1e-9);
    }

    #[test]
    fn unknown_model_is_unpriced() {
        let book = PriceBook::load(
            &CtxPaths::from_root(std::path::PathBuf::from("/tmp/ctx-no-prices")),
            "",
        );
        let row = with_price(&book, json!({"id": "__unknown__", "avoided": 1_000_000u64}));
        assert!(row["avoided_usd"].is_null());
        assert_eq!(row["price_estimate"], false);
        let money = price_summary(&[row]);
        assert!(money.avoided_usd.is_null());
        assert!(!money.estimated);
        assert_eq!(money.unpriced_models, 1);
    }

    #[test]
    fn quoted_models_ignore_unpriced_auto_rows_in_the_kpi() {
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
        assert!(auto["avoided_usd"].is_null());
        let money = price_summary(&[named, auto]);
        assert_eq!(money.avoided_usd, json!(3.0));
        assert!(!money.estimated);
        assert_eq!(money.priced_models, 1);
        assert_eq!(money.unpriced_models, 1);
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
