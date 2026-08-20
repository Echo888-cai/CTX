//! Deterministic context reduction.
//!
//! Level 1: regex / parsers / hashing — zero extra model tokens.
//! Level 2: store + handle (done by the runtime around these optimizers).
//! Level 3: semantic working set — pager, not these optimizers.

mod ansi;
mod budget;
mod cdc;
mod compact;
mod cow;
mod diff;
mod duplicate;
mod file_read;
mod frames;
mod generic;
mod git;
mod grep;
mod header;
mod install;
mod mcp;
mod pipeline;
mod plugin;
mod shell;
mod symbols;
mod tokens;

pub use budget::{
    cap as token_budget, cap_for, cap_hint, count_signal_lines, from_parts as budget_from_parts,
    lock_fill, BudgetHint, BudgetStrategy, MIN_GAIN_TOKENS,
};
pub use cdc::{cdc_working_set, chunk_text, Chunk};
pub use compact::{
    compact_block, diagnostic_preview, diagnostic_ranked, is_diagnostic_line, map_path_token,
    strip_backtraces,
};
pub use cow::cow_working_set;
pub use diff::diff_working_set;
pub use duplicate::DuplicateGuard;
pub use file_read::{extract_regions, outline_source, outline_working_set, ReadGuard};
pub use frames::{extract_frames, extract_map_hits, extract_maps, MapHit};
pub use generic::GenericGuard;
pub use header::prepend_command_exit;
pub use mcp::{reduce_json_like, McpGuard};
pub use pipeline::{OptimizeInput, OptimizeOutput, OptimizerSpec, Pipeline};
pub use plugin::{run_wasm_bytes, PluginGuard};
pub use shell::{reduce_shell, ShellGuard};
pub use symbols::{
    collect_symbol_spans, slice_span, symbol_at_line, symbol_label, symbol_name, SymbolSpan,
};
pub use tokens::{estimate_tokens, estimate_tokens_for, sniff_token_kind, TokenKind};
