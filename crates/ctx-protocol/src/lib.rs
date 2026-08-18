//! Harness-agnostic CTX event protocol.
//!
//! Adapters convert Claude Code, Cursor, and future harnesses into [`CtxEvent`].
//! Core never depends on harness-specific shapes.

mod event;
mod harness;
mod tool;
mod uri;

pub use event::*;
pub use harness::*;
pub use tool::*;
pub use uri::*;

/// Integer from hook JSON (`1`, `"1"`, or unsigned).
pub fn json_i64(v: &serde_json::Value) -> Option<i64> {
    v.as_i64()
        .or_else(|| v.as_u64().map(|n| n as i64))
        .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
}
