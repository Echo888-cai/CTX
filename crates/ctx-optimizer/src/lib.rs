//! Deterministic context reduction.
//!
//! Level 1: regex / parsers / hashing — zero extra model tokens.
//! Level 2: store + handle (done by the runtime around these optimizers).
//! Level 3: semantic working set — pager, not these optimizers.

mod ansi;
mod compact;
mod cow;
mod duplicate;
mod file_read;
mod frames;
mod generic;
mod header;
mod mcp;
mod pipeline;
mod shell;
mod symbols;
mod tokens;

pub use compact::{
    compact_block, diagnostic_preview, is_diagnostic_line, map_path_token, strip_backtraces,
};
pub use cow::cow_working_set;
pub use duplicate::DuplicateGuard;
pub use file_read::{extract_regions, outline_source, ReadGuard};
pub use frames::{extract_frames, extract_maps};
pub use generic::GenericGuard;
pub use header::prepend_command_exit;
pub use mcp::{reduce_json_like, McpGuard};
pub use pipeline::{OptimizeInput, OptimizeOutput, Pipeline};
pub use shell::{reduce_shell, ShellGuard};
pub use tokens::estimate_tokens;
