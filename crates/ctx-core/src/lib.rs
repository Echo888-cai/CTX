//! CTX runtime: ingest events, store raw context, deliver a working set.

mod canonical;
mod capability;
mod config;
mod economics;
mod execution;
mod format;
mod hooks;
mod overlay;
mod pagein;
mod intercept;
mod runtime;
mod spine;
mod wrap;

pub use canonical::{canonicalize_json, canonicalize_text, prefix_hash, with_cache_breakpoint};
pub use capability::{
    tools as capability_tools, tools_hash, wrap_tools, PROTOCOL_VERSION, TOOLS_VERSION,
};
pub use config::Config;
pub use economics::{advise as compact_advise, avoided_compact_usd, CompactAdvice};
pub use execution::evidence_pack;
pub use format::{
    ensure_exec_header, render_virtualized, render_virtualized_space, session_banner,
};
pub use hooks::{handle_hook, HookResponse};
pub use overlay::capture as capture_workspace;
pub use pagein::{bounded_preview, page_in, page_in_with_frames, FULL_PAGE_TOKENS};
pub use runtime::{IngestResult, Runtime};
pub use spine::{render_live as render_spine, Spine};
pub use wrap::{
    is_already_wrapped, resolve_ctx_bin, rewrite_shell_command, single_quote, wrap_shell_command,
    WrappedCommand,
};

pub use ctx_optimizer::estimate_tokens;
pub use ctx_pager::{
    extract_task, is_referenced, looks_signal, parse_task, start_of_today, start_of_week,
    RecentPage, WorkingSet,
};
pub use ctx_store::{hook_latency_ms, CtxPaths, ModelRow, Store, TokenTotals};
pub use ctx_telemetry::{
    catalog_json, fmt_compact, fmt_num, format_why, is_auto_id, official_price_meta,
    refresh_official_prices, refresh_official_prices_now, round_usd, session_report, PriceBook,
    PriceQuote, PriceSource, Snapshot,
};
