use ctx_optimizer::{
    cdc_working_set, chunk_text, cow_working_set, estimate_tokens, estimate_tokens_for,
    extract_frames, extract_map_hits, extract_regions, sniff_token_kind, symbol_at_line, Chunk,
    DuplicateGuard, MapHit, OptimizeInput, Pipeline, MIN_GAIN_TOKENS,
};
use ctx_pager::{extract_task, merge_tokens, parse_task, SemanticRanker, TfIdfRanker, WorkingSet};
use ctx_protocol::{CtxEvent, CtxUri, EventKind, Frame, ToolKind};
use ctx_store::{
    blake3_hex, digit_runs_differ, normalize_hash, simhash64, FileReadRecord, NewObservation,
    PutBlob, RecordPage, Store,
};

use crate::config::Config;
use crate::format::render_virtualized_space;
use crate::pagein::{
    bounded_preview_frames, first_snippet, frame_slice, match_score, page_in_with_frames,
};

pub struct Runtime {
    pub store: Store,
    pub config: Config,
    pipeline: Pipeline,
}

#[derive(Debug, Clone)]
pub struct IngestResult {
    pub delivered: String,
    pub replaced: bool,
    pub uri: Option<String>,
    pub raw_tokens: u32,
    pub delivered_tokens: u32,
    pub avoided_tokens: u32,
    pub optimizer: Option<String>,
    pub deduped: bool,
}

struct Finish<'a> {
    event: &'a CtxEvent,
    kind: &'a str,
    hash: &'a str,
    uri: Option<String>,
    delivered: String,
    optimizer: Option<&'a str>,
    replaced: bool,
    raw_tokens: u32,
}

struct Delivery {
    text: String,
    cow: bool,
}

impl Runtime {
    pub fn open(store: Store) -> Self {
        let config = Config::load(store.paths());
        let pipeline = Pipeline::from_specs(&config.optimizers);
        Self {
            store,
            config,
            pipeline,
        }
    }

    pub fn open_default() -> ctx_store::Result<Self> {
        Ok(Self::open(Store::open_default()?))
    }

    fn open_epoch_for(&self, event: &CtxEvent) -> ctx_store::Result<ctx_store::EpochRow> {
        let cwd = event
            .metadata
            .get("cwd")
            .and_then(|v| v.as_str())
            .map(std::path::Path::new);
        let (snap_id, n, manifest) = crate::overlay::capture(cwd);
        let _ = self.store.put_workspace_snapshot(&snap_id, n, &manifest);
        let model = event
            .metadata
            .get("model")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let tools = crate::capability::tools_hash();
        let prefix = crate::canonical::prefix_hash(&[
            crate::capability::protocol_text(),
            &snap_id,
            &tools,
        ]);
        let row = self.store.ensure_epoch(
            &event.session,
            model,
            "",
            &tools,
            crate::capability::PROTOCOL_VERSION,
            &snap_id,
            &prefix,
        )?;
        if self
            .store
            .journal_text(&event.session, row.epoch)
            .unwrap_or_default()
            .is_empty()
        {
            let _ = self.store.push_journal(&event.session, "epoch", &snap_id);
        }
        Ok(row)
    }

    pub fn is_harness_disabled(&self, harness: ctx_protocol::Harness) -> bool {
        self.config.is_harness_disabled(harness)
    }

    pub fn ingest(&self, event: CtxEvent) -> ctx_store::Result<IngestResult> {
        self.store.ensure_session_with_model(
            &event.session,
            event.harness.as_str(),
            event.metadata.get("cwd").and_then(|v| v.as_str()),
            event.metadata.get("model").and_then(|v| v.as_str()),
        )?;

        match event.event {
            EventKind::SessionStart => {
                let _ = self.open_epoch_for(&event);
                return Ok(passthrough(
                    mapped_greeting(&self.store, &event, 160, 6)?,
                    0,
                ));
            }
            EventKind::SessionEnd => {
                self.store.end_session(&event.session)?;
                return Ok(passthrough(String::new(), 0));
            }
            EventKind::PromptSubmit => {
                remember_task(&self.store, &event)?;
                if self.store.take_remap(&event.session)? {
                    return Ok(passthrough(
                        mapped_greeting(&self.store, &event, 160, 8)?,
                        0,
                    ));
                }
                return Ok(passthrough(String::new(), 0));
            }
            EventKind::Compact => {
                self.store.mark_remap(&event.session)?;
                let keep = event
                    .metadata
                    .get("keep")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                if keep {
                    return Ok(passthrough(
                        mapped_greeting(&self.store, &event, 160, 8)?,
                        0,
                    ));
                }
                return Ok(passthrough(String::new(), 0));
            }
            EventKind::ToolInput => {
                return Ok(passthrough(
                    event.payload.clone(),
                    estimate_tokens(&event.payload),
                ));
            }
            EventKind::ToolOutput | EventKind::FileRead => {}
        }

        if !self.config.enabled || self.config.is_harness_disabled(event.harness) {
            return Ok(passthrough(
                event.payload.clone(),
                estimate_tokens(&event.payload),
            ));
        }

        let kind = event
            .tool
            .as_ref()
            .map(|t| t.kind)
            .unwrap_or(ToolKind::Generic);
        let kind_str = kind.as_str();
        let raw_tokens = tokens_of(kind_str, &event.payload);

        let put = if event.payload.is_empty() {
            None
        } else {
            Some(
                self.store
                    .put_bytes_kind(event.payload.as_bytes(), Some(kind_str))?,
            )
        };
        let hash = put
            .as_ref()
            .map(|p| p.hash.clone())
            .unwrap_or_else(|| blake3_hex(event.payload.as_bytes()));
        let uri = CtxUri::new(kind_str, &hash);
        let frames =
            if kind == ToolKind::File || raw_tokens >= self.config.virtualize_threshold_tokens {
                extract_frames(kind_str, &event.payload)
            } else {
                Vec::new()
            };
        let task = ingest_task(&event, &frames);
        let task_s = task.join(" ");
        if !task.is_empty() {
            remember_tokens(&self.store, &event.session, &task)?;
        }
        let cwd = event.metadata.get("cwd").and_then(|v| v.as_str());
        let maps = self.mapped_pages(&extract_map_hits(&event.payload), cwd)?;

        let mut metadata = event.metadata.clone();
        if let Some(obj) = metadata.as_object_mut() {
            obj.insert("uri".into(), serde_json::Value::String(uri.to_string()));
            // Inline bodies from the prompt/session task, never from this page's own symbols.
            let inline = event
                .task_context
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .or_else(|| {
                    self.store
                        .session_task(&event.session)
                        .ok()
                        .filter(|s| !s.is_empty())
                });
            if let Some(task) = inline {
                obj.insert("task".into(), serde_json::Value::String(task));
            }
            let fetched = self.store.uri_was_fetched(&uri.page_key()).unwrap_or(false);
            obj.insert("fetched_before".into(), serde_json::Value::Bool(fetched));
            obj.insert(
                "budget_strategy".into(),
                serde_json::Value::String(self.config.budget_strategy.clone()),
            );
            let occupancy = self.store.session_occupancy_pct(&event.session).unwrap_or(0);
            let compacting = self.store.session_is_compacting(&event.session).unwrap_or(false);
            let tune = self.store.latest_optimizer_tune().unwrap_or(1.0);
            obj.insert(
                "occupancy_pct".into(),
                serde_json::Value::Number(occupancy.into()),
            );
            obj.insert("budget_tune".into(), serde_json::json!(tune));
            obj.insert("compacting".into(), serde_json::Value::Bool(compacting));
        }

        if kind == ToolKind::File {
            if let Some(path) = event.metadata.get("path").and_then(|v| v.as_str()) {
                let chunks = chunk_text(&event.payload);
                let chunks_json = serde_json::to_value(&chunks).unwrap_or_else(|_| serde_json::json!([]));
                if let Some(prev) = self.store.get_file_read(path)? {
                    if prev.content_hash != hash {
                        let _ = self.store.push_overlay(
                            &event.session,
                            path,
                            &prev.content_hash,
                            &hash,
                        );
                        let _ = self.store.push_journal(&event.session, "overlay", path);
                    }
                    if prev.content_hash == hash && raw_tokens >= 80 {
                        if let Some(obj) = metadata.as_object_mut() {
                            obj.insert("unchanged".into(), serde_json::Value::Bool(true));
                            obj.insert("regions".into(), prev.regions.clone());
                            if let Some(u) = &prev.last_uri {
                                obj.insert("uri".into(), u.clone().into());
                            }
                        }
                        let input = OptimizeInput {
                            kind: kind_str,
                            tool_name: event.tool.as_ref().map(|t| t.name.as_str()),
                            payload: &event.payload,
                            metadata: &metadata,
                            raw_tokens,
                        };
                        if let Some(out) = self.pipeline.run(&input) {
                            return self.finish(
                                Finish {
                                    event: &event,
                                    kind: kind_str,
                                    hash: &hash,
                                    uri: Some(uri.page_key()),
                                    delivered: out.text,
                                    optimizer: Some(out.optimizer),
                                    replaced: true,
                                    raw_tokens,
                                },
                                put.as_ref(),
                                &uri,
                                &frames,
                                &task_s,
                            );
                        }
                    } else if raw_tokens >= 80 {
                        let prev_chunks: Vec<Chunk> =
                            serde_json::from_value(prev.chunks.clone()).unwrap_or_default();
                        if let Some(delta) =
                            cdc_working_set(&prev_chunks, &chunks, &event.payload, &uri.to_string())
                        {
                            if tokens_of(kind_str, &delta) + MIN_GAIN_TOKENS < raw_tokens {
                                let regions = extract_regions(&event.payload);
                                self.store.upsert_file_read(&FileReadRecord {
                                    path: path.to_string(),
                                    content_hash: hash.clone(),
                                    last_uri: Some(uri.to_string()),
                                    last_tokens: raw_tokens,
                                    regions: serde_json::Value::Array(
                                        regions
                                            .into_iter()
                                            .map(serde_json::Value::String)
                                            .collect(),
                                    ),
                                    chunks: chunks_json.clone(),
                                })?;
                                return self.finish(
                                    Finish {
                                        event: &event,
                                        kind: kind_str,
                                        hash: &hash,
                                        uri: Some(uri.page_key()),
                                        delivered: delta,
                                        optimizer: Some("cdc"),
                                        replaced: true,
                                        raw_tokens,
                                    },
                                    put.as_ref(),
                                    &uri,
                                    &frames,
                                    &task_s,
                                );
                            }
                        }
                    }
                }
                let regions = extract_regions(&event.payload);
                self.store.upsert_file_read(&FileReadRecord {
                    path: path.to_string(),
                    content_hash: hash.clone(),
                    last_uri: Some(uri.to_string()),
                    last_tokens: raw_tokens,
                    regions: serde_json::Value::Array(
                        regions.into_iter().map(serde_json::Value::String).collect(),
                    ),
                    chunks: chunks_json,
                })?;
            }
        }

        let hit = if let Some(key) = event
            .metadata
            .get("dedup_key")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
        {
            if self.store.observation_exists_for_dedup(key)? {
                None
            } else {
                Some(self.remember_payload(&hash, &event.payload, &uri.to_string())?)
            }
        } else {
            Some(self.remember_payload(&hash, &event.payload, &uri.to_string())?)
        };

        if let Some(hit) = hit {
            if hit.count >= 2 && raw_tokens >= 80 {
                if let Some(text) = self.duplicate_stub(&hit, &uri.to_string(), kind_str, &event) {
                    return self.finish(
                        Finish {
                            event: &event,
                            kind: kind_str,
                            hash: &hash,
                            uri: Some(uri.page_key()),
                            delivered: text,
                            optimizer: Some("duplicate"),
                            replaced: true,
                            raw_tokens,
                        },
                        put.as_ref(),
                        &uri,
                        &frames,
                        &task_s,
                    );
                }
            }
        }

        if raw_tokens < self.config.virtualize_threshold_tokens {
            return self.finish(
                Finish {
                    event: &event,
                    kind: kind_str,
                    hash: &hash,
                    uri: Some(uri.page_key()),
                    delivered: event.payload.clone(),
                    optimizer: None,
                    replaced: false,
                    raw_tokens,
                },
                put.as_ref(),
                &uri,
                &frames,
                &task_s,
            );
        }

        let input = OptimizeInput {
            kind: kind_str,
            tool_name: event.tool.as_ref().map(|t| t.name.as_str()),
            payload: &event.payload,
            metadata: &metadata,
            raw_tokens,
        };

        if let Some(out) = self.pipeline.run(&input) {
            let mut text = out.text;
            if kind_str == "shell" && raw_tokens >= 400 {
                let pack = crate::evidence_pack(&event.payload, out.delivered_tokens.max(240));
                if crate::estimate_tokens(&pack) + MIN_GAIN_TOKENS < raw_tokens
                    && crate::estimate_tokens(&pack) < crate::estimate_tokens(&text)
                {
                    text = pack;
                }
            }
            let delivery = if out.duplicate_of.is_some() {
                Delivery {
                    text,
                    cow: false,
                }
            } else {
                self.pick_delivered(
                    &event,
                    kind_str,
                    &uri,
                    &hash,
                    raw_tokens,
                    text,
                    out.delivered_tokens,
                    out.optimizer,
                    &frames,
                    &maps,
                )?
            };
            let opt = if delivery.cow {
                Some("cow")
            } else {
                Some(out.optimizer)
            };
            return self.finish(
                Finish {
                    event: &event,
                    kind: kind_str,
                    hash: &hash,
                    uri: Some(uri.page_key()),
                    delivered: delivery.text,
                    optimizer: opt,
                    replaced: true,
                    raw_tokens,
                },
                put.as_ref(),
                &uri,
                &frames,
                &task_s,
            );
        }

        let delivery = self.pick_delivered(
            &event,
            kind_str,
            &uri,
            &hash,
            raw_tokens,
            event.payload.clone(),
            raw_tokens,
            "passthrough",
            &frames,
            &maps,
        )?;
        let replaced = delivery.cow || delivery.text != event.payload;
        self.finish(
            Finish {
                event: &event,
                kind: kind_str,
                hash: &hash,
                uri: Some(uri.page_key()),
                delivered: delivery.text,
                optimizer: if delivery.cow { Some("cow") } else { None },
                replaced,
                raw_tokens,
            },
            put.as_ref(),
            &uri,
            &frames,
            &task_s,
        )
    }

    fn finish(
        &self,
        f: Finish<'_>,
        blob: Option<&PutBlob>,
        page_uri: &CtxUri,
        frames: &[Frame],
        task: &str,
    ) -> ctx_store::Result<IngestResult> {
        let delivered_tokens = tokens_of(f.kind, &f.delivered);
        let avoided = f.raw_tokens.saturating_sub(delivered_tokens);
        let shadow = self.config.is_shadow(f.event.harness);
        let reasons = if let Some(name) = f.optimizer {
            serde_json::json!([{ "label": reason_label(name), "tokens": avoided }])
        } else {
            serde_json::json!([])
        };
        let source_path = f
            .event
            .metadata
            .get("path")
            .and_then(|v| v.as_str())
            .map(str::to_string);
        let exit_code = f
            .event
            .metadata
            .get("exit_code")
            .and_then(ctx_protocol::json_i64);
        let referenced = ctx_pager::is_referenced(
            Some(f.kind),
            f.event.tool.as_ref().map(|t| t.name.as_str()),
            &f.event.payload,
            exit_code,
            source_path.as_deref(),
            &[],
        );
        let dedup_key = f
            .event
            .metadata
            .get("dedup_key")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let id = self.store.commit_ingest(
            blob,
            Some(RecordPage {
                uri: page_uri,
                hash: f.hash,
                body: &f.event.payload,
                frames,
                raw_tokens: f.raw_tokens,
                harness: f.event.harness.as_str(),
                task,
            }),
            NewObservation {
                session_id: f.event.session.clone(),
                model: f
                    .event
                    .metadata
                    .get("model")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string(),
                event_type: f.event.event.as_str().to_string(),
                tool_type: Some(f.kind.to_string()),
                tool_name: f.event.tool.as_ref().map(|t| t.name.clone()),
                uri: f.uri.clone(),
                content_hash: f.hash.to_string(),
                raw_tokens: f.raw_tokens,
                delivered_tokens,
                avoided_tokens: avoided,
                optimizer: f.optimizer.map(|s| s.to_string()),
                reasons,
                referenced,
                source_path,
                dedup_key: dedup_key.clone(),
                shadow,
            },
        )?;
        if let Some(name) = f.optimizer {
            let _ = self.store.record_optimizer(name, avoided);
        }
        let model = f
            .event
            .metadata
            .get("model")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let _ = self
            .store
            .add_session_tokens(&f.event.session, delivered_tokens, window_for_model(model));
        self.schedule_prefetch(f.event, f.hash);
        let delivered = if shadow {
            f.event.payload.clone()
        } else {
            f.delivered
        };
        Ok(IngestResult {
            delivered,
            replaced: f.replaced && !shadow,
            uri: f.uri,
            raw_tokens: f.raw_tokens,
            delivered_tokens: if shadow { f.raw_tokens } else { delivered_tokens },
            avoided_tokens: avoided,
            optimizer: f.optimizer.map(|s| s.to_string()),
            deduped: id == 0 && !dedup_key.is_empty(),
        })
    }

    pub fn fetch(&self, uri: &str, query: Option<&str>) -> ctx_store::Result<String> {
        let started = std::time::Instant::now();
        let parsed =
            CtxUri::parse(uri).map_err(|e| ctx_store::StoreError::NotFound(e.to_string()))?;
        let key = parsed.page_key();
        let raw = String::from_utf8_lossy(&self.store.get_bytes_by_uri(&parsed)?).into_owned();
        let _ = self.store.touch_referenced(&key);
        let q = parsed
            .frame
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .or_else(|| query.map(str::trim).filter(|s| !s.is_empty()));
        let out: ctx_store::Result<String> = match q {
            Some("*" | "full") => Ok(raw),
            Some(name) => {
                if let Some(frame) = self.store.find_frame(&key, name)? {
                    Ok(frame_slice(
                        &raw,
                        &key,
                        &frame.name,
                        frame.start_line,
                        frame.end_line,
                    ))
                } else {
                    let frames = self.store.frames_for(&key).unwrap_or_default();
                    let listed: Vec<(String, u32, u32)> = frames
                        .into_iter()
                        .map(|f| (f.name, f.start_line, f.end_line))
                        .collect();
                    Ok(page_in_with_frames(&raw, name, &listed))
                }
            }
            None => {
                let frames = self.store.frames_for(&key).unwrap_or_default();
                let listed: Vec<(String, String, u32, u32)> = frames
                    .into_iter()
                    .map(|f| (f.name, f.kind, f.start_line, f.end_line))
                    .collect();
                Ok(bounded_preview_frames(&raw, &key, &listed))
            }
        };
        let body = out?;
        let fetched = estimate_tokens(&body);
        let _ = self.store.add_refetch(&key, fetched);
        ctx_store::record_page_fault(started.elapsed());
        Ok(body)
    }

    pub fn read_file(&self, path: &str, query: Option<&str>) -> anyhow::Result<String> {
        let bytes = std::fs::read(path)?;
        let text = String::from_utf8_lossy(&bytes).into_owned();
        match query.map(str::trim) {
            Some("*" | "full") => Ok(text),
            Some(q) if !q.is_empty() => {
                let spans = ctx_optimizer::collect_symbol_spans(&text);
                let frames: Vec<(String, u32, u32)> = spans
                    .into_iter()
                    .map(|s| (s.name, s.start_line, s.end_line))
                    .collect();
                Ok(page_in_with_frames(&text, q, &frames))
            }
            _ => {
                let tokens = estimate_tokens(&text);
                if tokens > self.config.large_file_tokens {
                    Ok(ctx_optimizer::outline_source(path, &text))
                } else {
                    Ok(text)
                }
            }
        }
    }

    /// Page-fault search: frame table first, then FTS, then lexical scan.
    pub fn search(&self, query: &str, limit: usize) -> ctx_store::Result<String> {
        let q = query.trim();
        if q.is_empty() {
            return Ok("query required — try ctx_search(\"error\") or ctx_inspect.\n".into());
        }
        let cap = limit.clamp(1, 16);
        let frames = self.store.search_frames(q, cap).unwrap_or_default();
        let fts = self.store.search_fts(q, cap).unwrap_or_default();
        if frames.is_empty() && fts.is_empty() {
            return self.search_scan(q, cap);
        }

        let mut out = format!("Page-fault search for {query:?}:\n\n");
        let mut n = 0usize;
        for hit in &frames {
            n += 1;
            let addr = if hit.name.is_empty() {
                hit.uri.clone()
            } else {
                format!("{}#{}", hit.uri, hit.name)
            };
            out.push_str(&format!("{n}. {addr}  {}\n", hit.kind));
            if !hit.hint.trim().is_empty() {
                out.push_str(&hit.hint);
                out.push('\n');
            }
            out.push('\n');
            if n >= cap {
                break;
            }
        }
        if n < cap {
            for hit in &fts {
                if frames.iter().any(|f| f.uri == hit.uri) {
                    continue;
                }
                n += 1;
                out.push_str(&format!(
                    "{n}. {}  {}  ~{} tokens\n",
                    hit.uri, hit.kind, hit.raw_tokens
                ));
                if !hit.snippet.trim().is_empty() {
                    out.push_str(&hit.snippet);
                    out.push('\n');
                }
                out.push('\n');
                if n >= cap {
                    break;
                }
            }
        }
        out.push_str("ctx_fetch uri#frame\n");
        Ok(out)
    }

    fn search_scan(&self, query: &str, cap: usize) -> ctx_store::Result<String> {
        let terms: Vec<&str> = query
            .split_whitespace()
            .map(|t| t.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '_' && c != '-'))
            .filter(|t| t.len() >= 2)
            .collect();
        let terms = if terms.is_empty() { vec![query] } else { terms };
        let pages = self.store.recent_pages(80)?;
        if pages.is_empty() {
            return Ok(
                "No pages stored yet. Run a command (or `ctx demo`) so CTX has something to page.\n"
                    .into(),
            );
        }
        let mut scored: Vec<(u32, String, String, u32, String)> = Vec::new();
        for page in pages {
            let Ok(bytes) = self.store.get_bytes(&page.hash) else {
                continue;
            };
            let text = String::from_utf8_lossy(&bytes);
            let score = match_score(&text, &terms);
            if score == 0 {
                continue;
            }
            let snippet = first_snippet(&text, &terms).unwrap_or_default();
            scored.push((score, page.uri, page.kind, page.raw_tokens, snippet));
        }
        scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
        scored.truncate(cap);
        if scored.is_empty() {
            return Ok(format!(
                "No stored pages matched {query:?}. Try a shorter token (error, FAIL, a file name).\n"
            ));
        }
        let mut out = format!(
            "Page-fault search for {query:?} ({} hits):\n\n",
            scored.len()
        );
        for (i, (score, uri, kind, tokens, snippet)) in scored.iter().enumerate() {
            out.push_str(&format!(
                "{}. {uri}  {kind}  ~{tokens} tokens  ({score} matching lines)\n",
                i + 1
            ));
            if !snippet.is_empty() {
                out.push_str(snippet);
                out.push('\n');
            }
            out.push('\n');
        }
        Ok(out)
    }

    #[allow(clippy::too_many_arguments)]
    fn pick_delivered(
        &self,
        event: &CtxEvent,
        kind: &str,
        uri: &CtxUri,
        hash: &str,
        raw_tokens: u32,
        candidate: String,
        candidate_tokens: u32,
        optimizer: &str,
        frames: &[Frame],
        maps: &[String],
    ) -> ctx_store::Result<Delivery> {
        if let Some((cow_text, _prev)) =
            self.cow_text(&event.session, kind, &uri.page_key(), hash, &event.payload)?
        {
            let cow_tokens = estimate_tokens(&cow_text);
            if cow_tokens + 40 < candidate_tokens {
                return Ok(Delivery {
                    text: render_virtualized_space(
                        &cow_text, uri, raw_tokens, cow_tokens, frames, maps,
                    ),
                    cow: true,
                });
            }
        }
        if optimizer == "passthrough" {
            return Ok(Delivery {
                text: candidate,
                cow: false,
            });
        }
        Ok(Delivery {
            text: render_virtualized_space(
                &candidate,
                uri,
                raw_tokens,
                candidate_tokens,
                frames,
                maps,
            ),
            cow: false,
        })
    }

    fn cow_text(
        &self,
        session: &str,
        kind: &str,
        uri: &str,
        hash: &str,
        payload: &str,
    ) -> ctx_store::Result<Option<(String, String)>> {
        let Some((prev_uri, prev_hash)) = self.store.last_page_for(session, kind, uri)? else {
            return Ok(None);
        };
        if prev_hash == hash {
            return Ok(None);
        }
        let Ok(bytes) = self.store.get_bytes(&prev_hash) else {
            return Ok(None);
        };
        let prev = String::from_utf8_lossy(&bytes);
        Ok(cow_working_set(&prev, &prev_uri, payload).map(|text| (text, prev_uri)))
    }

    fn schedule_prefetch(&self, event: &CtxEvent, hash: &str) {
        let mut hashes = Vec::new();
        for hit in extract_map_hits(&event.payload) {
            if let Ok(Some(prev)) = self.store.get_file_read(&hit.path) {
                if prev.content_hash != hash {
                    hashes.push(prev.content_hash);
                }
            }
        }
        let task = parse_task(&self.store.session_task(&event.session).unwrap_or_default());
        if !task.is_empty() {
            if let Ok(pages) = self.store.recent_pages(48) {
                for (i, score) in TfIdfRanker.rank(&task, &pages) {
                    if score <= 0.01 {
                        break;
                    }
                    if pages[i].hash != hash {
                        hashes.push(pages[i].hash.clone());
                    }
                    if hashes.len() >= 4 {
                        break;
                    }
                }
            }
        }
        hashes.retain(|h| h != hash);
        hashes.truncate(4);
        self.store.prefetch(&hashes);
    }

    fn mapped_pages(&self, hits: &[MapHit], cwd: Option<&str>) -> ctx_store::Result<Vec<String>> {
        let mut out = Vec::new();
        for hit in hits {
            let symbol = self.symbol_for_hit(hit, cwd)?;
            let loc = match (hit.line, symbol.as_deref()) {
                (Some(n), Some(s)) => format!("{}:{n}#{s}", hit.path),
                (Some(n), None) => format!("{}:{n}", hit.path),
                (None, Some(s)) => format!("{}#{s}", hit.path),
                (None, None) => hit.path.clone(),
            };
            if let Some(prev) = self.store.get_file_read(&hit.path)? {
                if let Some(uri) = prev.last_uri {
                    if let Some(s) = symbol {
                        out.push(format!("{uri}#{s}  {loc}"));
                    } else {
                        out.push(format!("{uri}  {loc}"));
                    }
                    continue;
                }
            }
            out.push(format!("{loc}  (ctx_read)"));
        }
        Ok(out)
    }

    fn symbol_for_hit(&self, hit: &MapHit, cwd: Option<&str>) -> ctx_store::Result<Option<String>> {
        let line = match hit.line {
            Some(n) => n,
            None => return Ok(None),
        };
        if let Some(prev) = self.store.get_file_read(&hit.path)? {
            if let Some(uri) = &prev.last_uri {
                if let Ok(frames) = self.store.frames_for(uri) {
                    if let Some(name) = enclosing_symbol(&frames, line) {
                        return Ok(Some(name));
                    }
                }
            }
            if let Ok(bytes) = self.store.get_bytes(&prev.content_hash) {
                let src = String::from_utf8_lossy(&bytes);
                return Ok(symbol_at_line(&src, line));
            }
        }
        Ok(symbol_from_disk(cwd, &hit.path, line))
    }

    fn remember_payload(
        &self,
        hash: &str,
        payload: &str,
        uri: &str,
    ) -> ctx_store::Result<ctx_store::FingerprintHit> {
        let sim = if payload.len() > 2 * 1024 * 1024 {
            0
        } else {
            simhash64(payload)
        };
        self.store.remember_fingerprint_near(
            hash,
            &normalize_hash(payload),
            Some(uri),
            sim,
            self.config.near_duplicate_hamming,
        )
    }

    /// Exact / whitespace dups collapse. Near-dups that change digit runs
    /// (status codes, assertion values) must not — that is signal.
    fn duplicate_stub(
        &self,
        hit: &ctx_store::FingerprintHit,
        uri: &str,
        kind: &str,
        event: &CtxEvent,
    ) -> Option<String> {
        let ref_uri = hit.uri.clone().unwrap_or_else(|| uri.to_string());
        let tool = event.tool.as_ref().map(|t| t.name.as_str());
        if !hit.near {
            return Some(DuplicateGuard::render(&ref_uri, hit.count, kind, tool));
        }
        let Ok(bytes) = self.store.get_bytes(&hit.hash) else {
            return None;
        };
        let prev = String::from_utf8_lossy(&bytes);
        if digit_runs_differ(&prev, &event.payload) {
            return None;
        }
        let delta = DuplicateGuard::brief_delta(&prev, &event.payload, 6);
        Some(DuplicateGuard::render_near(
            &ref_uri,
            hit.count,
            kind,
            tool,
            Some(hit.hamming),
            Some(delta.as_str()).filter(|s| !s.is_empty()),
        ))
    }
}

fn enclosing_symbol(frames: &[Frame], line: u32) -> Option<String> {
    frames
        .iter()
        .filter(|f| f.start_line <= line && line <= f.end_line)
        .min_by_key(|f| f.end_line.saturating_sub(f.start_line))
        .map(|f| f.name.clone())
}

fn symbol_from_disk(cwd: Option<&str>, path: &str, line: u32) -> Option<String> {
    let p = std::path::Path::new(path);
    let full = if p.is_absolute() {
        p.to_path_buf()
    } else if let Some(c) = cwd {
        std::path::Path::new(c).join(p)
    } else {
        p.to_path_buf()
    };
    let meta = std::fs::metadata(&full).ok()?;
    if meta.len() > 2_000_000 {
        return None;
    }
    let src = std::fs::read_to_string(full).ok()?;
    symbol_at_line(&src, line)
}

fn remember_task(store: &Store, event: &CtxEvent) -> ctx_store::Result<()> {
    let tc = event.task_context.as_deref().unwrap_or("");
    let tokens = extract_task(&[event.payload.as_str(), tc]);
    remember_tokens(store, &event.session, &tokens)
}

fn remember_tokens(store: &Store, session: &str, tokens: &[String]) -> ctx_store::Result<()> {
    if tokens.is_empty() {
        return Ok(());
    }
    let stored = parse_task(&store.session_task(session)?);
    let merged = merge_tokens(&stored, tokens);
    store.set_session_task(session, &merged.join(" "))
}

fn ingest_task(event: &CtxEvent, frames: &[Frame]) -> Vec<String> {
    let tc = event.task_context.as_deref().unwrap_or("");
    let cmd = event
        .metadata
        .get("command")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let path = event
        .metadata
        .get("path")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let mut parts: Vec<&str> = vec![tc, cmd, path];
    for f in frames {
        parts.push(&f.name);
    }
    extract_task(&parts)
}

fn mapped_greeting(
    store: &Store,
    event: &CtxEvent,
    budget: u32,
    pages: usize,
) -> ctx_store::Result<String> {
    let cwd = event
        .metadata
        .get("cwd")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let base = std::path::Path::new(cwd)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    let extra = extract_task(&[base]);
    let ws = WorkingSet::query(store, Some(&event.session), &extra)?;
    let mut greet = trim_greeting(&ws, budget, pages);
    if let Ok(Some(ep)) = store.current_epoch(&event.session) {
        greet = format!(
            "{greet}\nEpoch {}  BASE {}\n",
            ep.epoch,
            if ep.workspace_snapshot.is_empty() {
                "—"
            } else {
                &ep.workspace_snapshot
            }
        );
    }
    Ok(greet)
}

fn trim_greeting(ws: &WorkingSet, budget: u32, pages: usize) -> String {
    let banner = crate::session_banner();
    let caps: Vec<usize> = [pages, 4, 2].into_iter().filter(|n| *n > 0).collect();
    for n in caps {
        let mut slim = ws.clone();
        slim.recent_pages.truncate(n);
        let mapped = slim.render_mapped();
        if mapped.is_empty() {
            break;
        }
        if estimate_tokens(&mapped) <= budget {
            return format!("{banner}\n\n{mapped}");
        }
    }
    banner.to_string()
}

fn tokens_of(kind: &str, text: &str) -> u32 {
    estimate_tokens_for(sniff_token_kind(kind, text), text)
}

fn window_for_model(model: &str) -> u32 {
    let m = model.to_ascii_lowercase();
    if m.contains("opus")
        || m.contains("sonnet")
        || m.contains("gpt-5")
        || m.contains("gpt-4.1")
        || m.contains("gpt-4o")
    {
        200_000
    } else {
        128_000
    }
}

fn passthrough(delivered: String, tokens: u32) -> IngestResult {
    IngestResult {
        delivered,
        replaced: false,
        uri: None,
        raw_tokens: tokens,
        delivered_tokens: tokens,
        avoided_tokens: 0,
        optimizer: None,
        deduped: false,
    }
}

fn reason_label(optimizer: &str) -> &'static str {
    match optimizer {
        "cow" => "copy-on-write delta",
        "shell" => "test output noise",
        "file-read" => "duplicate file reads",
        "duplicate" => "repeated tool output",
        "mcp" => "mcp json payload",
        _ => "irrelevant log regions",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ctx_protocol::{Harness, ToolRef};
    use ctx_store::CtxPaths;

    #[test]
    fn virtualizes_noisy_shell_and_fetches_raw() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(CtxPaths::from_root(dir.path().to_path_buf())).unwrap();
        let rt = Runtime::open(store);
        let mut payload = String::from("running 400 tests\n");
        for i in 0..400 {
            payload.push_str(&format!("test t{i} ... ok\n"));
        }
        payload.push_str("test auth::login ... FAILED\n\nfailures:\n\n---- auth::login stdout ----\nleft: 401\nright: 200\n\ntest result: FAILED. 400 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1s\n");
        let event = CtxEvent::tool_output(
            "s1",
            Harness::ClaudeCode,
            ToolRef::new("Bash"),
            payload.clone(),
        );
        let result = rt.ingest(event).unwrap();
        assert!(result.replaced);
        assert!(result.avoided_tokens > 0);
        assert!(result.delivered.contains("ctx://"));
        assert!(result.delivered.contains("401"));
        let obs = rt.store.observations_for_session("s1").unwrap();
        assert!(
            obs.iter().any(|o| o.referenced),
            "FAILED shell output should set the clock bit"
        );
        let uri = result.uri.unwrap();
        let raw = rt.fetch(&uri, Some("*")).unwrap();
        assert_eq!(raw, payload);
        let page = rt.fetch(&uri, Some("401")).unwrap();
        assert!(page.contains("401"));
        let preview = rt.fetch(&uri, None).unwrap();
        assert!(
            preview.contains(&uri) || preview.len() < payload.len(),
            "{preview}"
        );
        let found = rt.search("401", 8).unwrap();
        assert!(found.contains(&uri), "{found}");
        let by_frame = rt.search("auth::login", 8).unwrap();
        assert!(
            by_frame.contains(&format!("{uri}#auth::login")),
            "{by_frame}"
        );
        let framed = rt.fetch(&format!("{uri}#auth::login"), None).unwrap();
        assert!(framed.contains("401"), "{framed}");
        assert!(framed.contains("auth::login"), "{framed}");
        assert!(
            !framed.contains("foo::bar ... ok"),
            "frame walk must not dump passing tests:\n{framed}"
        );
        assert!(
            result.delivered.contains("#auth"),
            "{delivered}",
            delivered = result.delivered
        );
        assert!(
            !result.delivered.contains("No context was lost"),
            "envelope copy must not tax the working set:\n{}",
            result.delivered
        );
        assert!(
            !result.delivered.contains("Address space"),
            "{}",
            result.delivered
        );
        assert!(
            result.delivered_tokens * 4 < result.raw_tokens,
            "raw={} delivered={}",
            result.raw_tokens,
            result.delivered_tokens
        );
        let obs_after = rt.store.observations_for_session("s1").unwrap();
        assert!(
            obs_after
                .iter()
                .any(|o| o.referenced && o.uri.as_deref() == Some(uri.as_str())),
            "fetch should set the clock referenced bit"
        );
        let pages = rt.store.recent_pages(8).unwrap();
        assert_eq!(pages[0].harness, "claude-code");
        assert!(pages[0].task.contains("auth"), "{:?}", pages[0].task);
    }

    #[test]
    fn cursor_inspect_sees_claude_pages() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(CtxPaths::from_root(dir.path().to_path_buf())).unwrap();
        let rt = Runtime::open(store);
        let mut payload = String::from("running 400 tests\n");
        for i in 0..400 {
            payload.push_str(&format!("test t{i} ... ok\n"));
        }
        payload.push_str("test auth::login ... FAILED\nleft: 401\n");
        rt.ingest(CtxEvent::tool_output(
            "claude-s",
            Harness::ClaudeCode,
            ToolRef::new("Bash"),
            payload,
        ))
        .unwrap();
        rt.store.ensure_session("cursor-s", "cursor", None).unwrap();
        rt.store.set_session_task("cursor-s", "auth login").unwrap();
        let ws = ctx_pager::WorkingSet::query(&rt.store, Some("cursor-s"), &[]).unwrap();
        assert!(
            ws.recent_pages.iter().any(|p| p.harness == "claude-code"),
            "{:?}",
            ws.recent_pages
        );
        assert!(
            ws.recent_pages
                .iter()
                .any(|p| p.uri.contains("shell") && p.frame.contains("auth")),
            "{:?}",
            ws.recent_pages
        );
    }

    #[test]
    fn small_fetch_without_query_is_full_page() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(CtxPaths::from_root(dir.path().to_path_buf())).unwrap();
        let rt = Runtime::open(store);
        let event = CtxEvent::tool_output(
            "s2",
            Harness::ClaudeCode,
            ToolRef::new("Bash"),
            "hello from ctx".to_string(),
        );
        let result = rt.ingest(event).unwrap();
        let uri = result.uri.unwrap();
        let got = rt.fetch(&uri, None).unwrap();
        assert_eq!(got.trim(), "hello from ctx");
    }

    #[test]
    fn virtualizes_medium_source_file() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(CtxPaths::from_root(dir.path().to_path_buf())).unwrap();
        let rt = Runtime::open(store);
        let mut src = String::from("use std::io;\n\n");
        for i in 0..50 {
            src.push_str(&format!("pub fn thing_{i}(x: i32) -> i32 {{ x + {i} }}\n"));
        }
        let event = CtxEvent {
            event: EventKind::FileRead,
            session: "s-file".into(),
            harness: Harness::ClaudeCode,
            tool: Some(ToolRef::new("Read")),
            payload: src.clone(),
            task_context: None,
            metadata: serde_json::json!({"path": "src/lib.rs"}),
        };
        let result = rt.ingest(event).unwrap();
        assert!(result.replaced, "{}", result.delivered);
        assert!(
            result.delivered.contains("fn thing_0"),
            "{}",
            result.delivered
        );
        assert!(!result.delivered.contains("x + 12"), "{}", result.delivered);
        assert!(result.delivered.contains("ctx://"), "{}", result.delivered);
        assert!(result.delivered_tokens + 80 < result.raw_tokens);
    }

    #[test]
    fn fetch_named_file_frame_returns_body() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(CtxPaths::from_root(dir.path().to_path_buf())).unwrap();
        let rt = Runtime::open(store);
        let mut padded = String::from("use std::io;\n\n");
        padded.push_str("fn noise() {\n    1\n}\n");
        padded
            .push_str("pub fn login(user: &str) -> i32 {\n    let status = 401;\n    status\n}\n");
        padded.push_str("fn tail() { 2 }\n");
        for i in 0..40 {
            padded.push_str(&format!("fn extra_{i}() {{ {i} }}\n"));
        }
        let event = CtxEvent {
            event: EventKind::FileRead,
            session: "s-fetch".into(),
            harness: Harness::ClaudeCode,
            tool: Some(ToolRef::new("Read")),
            payload: padded,
            task_context: Some("fix login".into()),
            metadata: serde_json::json!({"path": "src/auth.rs"}),
        };
        let result = rt.ingest(event).unwrap();
        let uri = result.uri.expect("uri");
        let page = rt.fetch(&format!("{uri}#login"), None).unwrap();
        assert!(page.contains("let status = 401"), "{page}");
        assert!(page.contains("login"), "{page}");
        assert!(!page.contains("fn extra_12"), "{page}");
    }

    #[test]
    fn compiler_span_prefetches_file_frame() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(CtxPaths::from_root(dir.path().to_path_buf())).unwrap();
        let rt = Runtime::open(store);
        let mut src = String::from("fn noise() { 1 }\n");
        src.push_str("pub fn login(x: i32) -> i32 {\n    401\n}\n");
        src.push_str("fn tail() { 2 }\n");
        for i in 0..40 {
            src.push_str(&format!("fn extra_{i}() {{ {i} }}\n"));
        }
        let read = CtxEvent {
            event: EventKind::FileRead,
            session: "s-map".into(),
            harness: Harness::ClaudeCode,
            tool: Some(ToolRef::new("Read")),
            payload: src,
            task_context: None,
            metadata: serde_json::json!({"path": "src/auth.rs"}),
        };
        let file = rt.ingest(read).unwrap();
        let file_uri = file.uri.expect("file uri");
        let compile = CtxEvent {
            event: EventKind::ToolOutput,
            session: "s-map".into(),
            harness: Harness::ClaudeCode,
            tool: Some(ToolRef::new("Bash")),
            payload: {
                let mut p = String::from("error[E0308]: mismatched types\n  --> src/auth.rs:3:5\n");
                for i in 0..80 {
                    p.push_str(&format!("Compiling crate{i} v1.0.0\n"));
                }
                p
            },
            task_context: None,
            metadata: serde_json::json!({"command": "cargo build", "exit_code": 101}),
        };
        let out = rt.ingest(compile).unwrap();
        assert!(
            out.delivered.contains(&format!("{file_uri}#login"))
                || (out.delivered.contains("src/auth.rs") && out.delivered.contains("#login")),
            "{}",
            out.delivered
        );
    }

    #[test]
    fn tiny_shell_skips_frame_extract() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(CtxPaths::from_root(dir.path().to_path_buf())).unwrap();
        let rt = Runtime::open(store);
        let event =
            CtxEvent::tool_output("s-tiny", Harness::ClaudeCode, ToolRef::new("Bash"), "ok\n");
        let result = rt.ingest(event).unwrap();
        let uri = result.uri.expect("uri");
        assert!(
            rt.store.frames_for(&uri).unwrap().is_empty(),
            "small payloads should skip frame extraction"
        );
    }

    fn noisy_log() -> String {
        let mut payload = String::from("running 400 tests\n");
        for i in 0..400 {
            payload.push_str(&format!("test t{i} ... ok\n"));
        }
        payload.push_str("test auth::login ... FAILED\n\nfailures:\n\n---- auth::login stdout ----\nleft: 401\nright: 200\n");
        payload
    }

    #[test]
    fn refetch_is_netted_out_of_savings() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(CtxPaths::from_root(dir.path().to_path_buf())).unwrap();
        let rt = Runtime::open(store);
        let result = rt
            .ingest(CtxEvent::tool_output(
                "s-net",
                Harness::ClaudeCode,
                ToolRef::new("Bash"),
                noisy_log(),
            ))
            .unwrap();
        assert!(result.avoided_tokens > 0);
        let gross = result.avoided_tokens as u64;
        let before = rt.store.totals_since(0).unwrap();
        assert_eq!(before.avoided, gross);
        assert_eq!(before.net_avoided(), gross);
        let uri = result.uri.expect("uri");
        let fetched = rt.fetch(&uri, Some("*")).unwrap();
        let refetched = estimate_tokens(&fetched) as u64;
        assert!(refetched > 0, "full fetch should return original tokens");
        let after = rt.store.totals_since(0).unwrap();
        assert_eq!(after.avoided, gross, "gross savings stay");
        assert_eq!(after.refetched, refetched);
        assert_eq!(after.net_avoided(), gross.saturating_sub(refetched));
    }

    #[test]
    fn shadow_mode_reports_savings_without_changing_delivery() {
        let dir = tempfile::tempdir().unwrap();
        let paths = CtxPaths::from_root(dir.path().to_path_buf());
        let mut cfg = Config::default();
        cfg.shadow_mode = true;
        cfg.save(&paths).unwrap();
        let store = Store::open(paths).unwrap();
        let rt = Runtime::open(store);
        let payload = noisy_log();
        let result = rt
            .ingest(CtxEvent::tool_output(
                "s-shadow",
                Harness::ClaudeCode,
                ToolRef::new("Bash"),
                payload.clone(),
            ))
            .unwrap();
        assert_eq!(result.delivered, payload);
        assert!(!result.replaced);
        assert!(result.avoided_tokens > 0);
        let obs = rt.store.observations_for_session("s-shadow").unwrap();
        assert!(obs.iter().any(|o| o.avoided_tokens > 0), "{obs:?}");
    }

    #[test]
    fn disabled_harness_passes_through() {
        let dir = tempfile::tempdir().unwrap();
        let paths = CtxPaths::from_root(dir.path().to_path_buf());
        let mut cfg = Config::default();
        cfg.disabled_harnesses = vec!["claude-code".into()];
        cfg.save(&paths).unwrap();
        let store = Store::open(paths).unwrap();
        let rt = Runtime::open(store);
        let payload = noisy_log();
        let result = rt
            .ingest(CtxEvent::tool_output(
                "s-off",
                Harness::ClaudeCode,
                ToolRef::new("Bash"),
                payload.clone(),
            ))
            .unwrap();
        assert_eq!(result.delivered, payload);
        assert!(!result.replaced);
        assert_eq!(result.avoided_tokens, 0);
        assert!(rt.store.observations_for_session("s-off").unwrap().is_empty());
    }

    #[test]
    fn edited_file_uses_cdc_delta_when_chunks_overlap() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(CtxPaths::from_root(dir.path().to_path_buf())).unwrap();
        let rt = Runtime::open(store);
        let mut src = "hello world\n".repeat(900);
        src.push_str("pub fn keep() { 1 }\n");
        let first = CtxEvent {
            event: EventKind::FileRead,
            session: "s-cdc".into(),
            harness: Harness::ClaudeCode,
            tool: Some(ToolRef::new("Read")),
            payload: src.clone(),
            task_context: None,
            metadata: serde_json::json!({"path": "src/big.rs"}),
        };
        rt.ingest(first).unwrap();
        src.replace_range(24..29, "HELLO");
        let second = CtxEvent {
            event: EventKind::FileRead,
            session: "s-cdc".into(),
            harness: Harness::ClaudeCode,
            tool: Some(ToolRef::new("Read")),
            payload: src,
            task_context: None,
            metadata: serde_json::json!({"path": "src/big.rs"}),
        };
        let out = rt.ingest(second).unwrap();
        assert!(out.replaced, "{}", out.delivered);
        assert!(
            out.optimizer.as_deref() == Some("cdc") || out.delivered.contains("ctx://"),
            "opt={:?} delivered={}",
            out.optimizer,
            out.delivered
        );
        assert!(out.delivered_tokens + 40 < out.raw_tokens);
    }

    #[test]
    fn default_near_duplicate_keeps_status_code_changes() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(CtxPaths::from_root(dir.path().to_path_buf())).unwrap();
        let rt = Runtime::open(store);
        assert_eq!(rt.config.near_duplicate_hamming, 0);
        let a = "error: boom\nleft: 401\nright: 200\n".repeat(40);
        let b = a.replace("401", "402");
        rt.ingest(CtxEvent::tool_output(
            "s-near",
            Harness::ClaudeCode,
            ToolRef::new("Bash"),
            a,
        ))
        .unwrap();
        let second = rt
            .ingest(CtxEvent::tool_output(
                "s-near",
                Harness::ClaudeCode,
                ToolRef::new("Bash"),
                b,
            ))
            .unwrap();
        assert_ne!(
            second.optimizer.as_deref(),
            Some("duplicate"),
            "{}",
            second.delivered
        );
        assert!(second.delivered.contains("402"), "{}", second.delivered);
    }

    #[test]
    fn simhash_near_duplicate_with_digit_change_is_kept() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(CtxPaths::from_root(dir.path().to_path_buf())).unwrap();
        let mut rt = Runtime::open(store);
        rt.config.near_duplicate_hamming = 3;
        let a = "error: boom\nleft: 401\nright: 200\n".repeat(40);
        let b = a.replace("401", "402");
        rt.ingest(CtxEvent::tool_output(
            "s-near-h",
            Harness::ClaudeCode,
            ToolRef::new("Bash"),
            a,
        ))
        .unwrap();
        let second = rt
            .ingest(CtxEvent::tool_output(
                "s-near-h",
                Harness::ClaudeCode,
                ToolRef::new("Bash"),
                b,
            ))
            .unwrap();
        assert_ne!(
            second.optimizer.as_deref(),
            Some("duplicate"),
            "digit-run changes must not collapse:\n{}",
            second.delivered
        );
    }

    #[test]
    fn exact_duplicate_still_collapses() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(CtxPaths::from_root(dir.path().to_path_buf())).unwrap();
        let rt = Runtime::open(store);
        let payload = "error: boom\nleft: 401\nright: 200\n".repeat(40);
        rt.ingest(CtxEvent::tool_output(
            "s-exact",
            Harness::ClaudeCode,
            ToolRef::new("Bash"),
            payload.clone(),
        ))
        .unwrap();
        let second = rt
            .ingest(CtxEvent::tool_output(
                "s-exact",
                Harness::ClaudeCode,
                ToolRef::new("Bash"),
                payload,
            ))
            .unwrap();
        assert_eq!(second.optimizer.as_deref(), Some("duplicate"), "{}", second.delivered);
        assert!(second.delivered.contains("dup"), "{}", second.delivered);
    }

    #[test]
    fn session_start_opens_epoch_one() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(CtxPaths::from_root(dir.path().to_path_buf())).unwrap();
        let rt = Runtime::open(store);
        rt.ingest(CtxEvent {
            event: EventKind::SessionStart,
            session: "s-ep".into(),
            harness: Harness::ClaudeCode,
            tool: None,
            payload: String::new(),
            task_context: None,
            metadata: serde_json::json!({"model": "claude-sonnet-4"}),
        })
        .unwrap();
        let ep = rt.store.current_epoch("s-ep").unwrap().expect("epoch");
        assert_eq!(ep.epoch, 1);
        assert_eq!(ep.model, "claude-sonnet-4");
    }

    #[test]
    fn file_hash_change_appends_overlay() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(CtxPaths::from_root(dir.path().to_path_buf())).unwrap();
        let rt = Runtime::open(store);
        rt.ingest(CtxEvent {
            event: EventKind::SessionStart,
            session: "s-ov".into(),
            harness: Harness::ClaudeCode,
            tool: None,
            payload: String::new(),
            task_context: None,
            metadata: serde_json::json!({}),
        })
        .unwrap();
        let mut src = String::from("fn a() { 1 }\n");
        for i in 0..40 {
            src.push_str(&format!("fn n{i}() {{ {i} }}\n"));
        }
        rt.ingest(CtxEvent {
            event: EventKind::FileRead,
            session: "s-ov".into(),
            harness: Harness::ClaudeCode,
            tool: Some(ToolRef::new("Read")),
            payload: src.clone(),
            task_context: None,
            metadata: serde_json::json!({"path": "src/a.rs"}),
        })
        .unwrap();
        src.push_str("fn changed() { 9 }\n");
        rt.ingest(CtxEvent {
            event: EventKind::FileRead,
            session: "s-ov".into(),
            harness: Harness::ClaudeCode,
            tool: Some(ToolRef::new("Read")),
            payload: src,
            task_context: None,
            metadata: serde_json::json!({"path": "src/a.rs"}),
        })
        .unwrap();
        let rows = rt.store.overlays_for("s-ov", 1).unwrap();
        assert_eq!(rows.len(), 1, "{rows:?}");
        assert_eq!(rows[0].path, "src/a.rs");
        assert_ne!(rows[0].prev_hash, rows[0].new_hash);
    }
}
