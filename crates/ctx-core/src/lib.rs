//! CTX runtime: ingest events, store raw context, deliver a working set.

mod config;
mod format;
mod hooks;
mod pagein;
mod runtime;
mod wrap;

pub use config::Config;
pub use format::{
    ensure_exec_header, render_virtualized, render_virtualized_space, session_banner,
};
pub use hooks::{handle_hook, HookResponse};
pub use pagein::{bounded_preview, page_in, page_in_with_frames, FULL_PAGE_TOKENS};
pub use runtime::{IngestResult, Runtime};
pub use wrap::{
    is_already_wrapped, resolve_ctx_bin, rewrite_shell_command, single_quote, wrap_shell_command,
    WrappedCommand,
};

pub use ctx_optimizer::estimate_tokens;
pub use ctx_pager::{
    extract_task, is_referenced, looks_signal, parse_task, start_of_today, start_of_week,
    RecentPage, WorkingSet,
};
pub use ctx_store::{CtxPaths, Store, TokenTotals};
pub use ctx_telemetry::{fmt_compact, fmt_num, format_why, session_report, Snapshot};
