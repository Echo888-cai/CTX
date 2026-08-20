//! L0–L4 cache spine. Prefix (L0+L1) is frozen for the epoch.

use serde::Serialize;
use serde_json::{json, Value};

use crate::canonical::{canonicalize_json, canonicalize_text, prefix_hash};
use crate::capability::protocol_text;
use ctx_store::{EpochRow, OverlayRow};

#[derive(Debug, Clone, Serialize)]
pub struct Spine {
    pub l0: String,
    pub l1: String,
    pub l2: String,
    pub l3: String,
    pub l4: String,
    pub prefix_hash: String,
}

impl Spine {
    pub fn assemble(
        snapshot_id: &str,
        overlays: &[OverlayRow],
        journal: &str,
        working_set: &str,
        current_turn: &str,
    ) -> Self {
        let l0 = protocol_text().to_string();
        let l1 = render_snapshot(snapshot_id, overlays);
        let prefix_hash = prefix_hash(&[&l0, &l1]);
        Self {
            l0,
            l1,
            l2: journal.trim().to_string(),
            l3: working_set.trim().to_string(),
            l4: current_turn.trim().to_string(),
            prefix_hash,
        }
    }

    pub fn from_epoch(
        epoch: &EpochRow,
        overlays: &[OverlayRow],
        journal: &str,
        working_set: &str,
        turn: &str,
    ) -> Self {
        Self::assemble(
            &epoch.workspace_snapshot,
            overlays,
            journal,
            working_set,
            turn,
        )
    }

    /// Frozen prefix above the cache line.
    pub fn prefix(&self) -> String {
        format!("{}\n\n{}\n", self.l0.trim(), self.l1.trim())
    }

    pub fn tail(&self) -> String {
        let mut out = String::new();
        if !self.l2.is_empty() {
            out.push_str(&self.l2);
            out.push('\n');
        }
        if !self.l3.is_empty() {
            out.push_str(&self.l3);
            out.push('\n');
        }
        if !self.l4.is_empty() {
            out.push_str(&self.l4);
            out.push('\n');
        }
        out
    }

    pub fn render(&self) -> String {
        format!(
            "{}\n--- CACHE LINE {} ---\n{}",
            self.prefix().trim_end(),
            &self.prefix_hash[..12.min(self.prefix_hash.len())],
            self.tail()
        )
    }

    /// Anthropic `system` array: L0+L1 with a cache breakpoint, then uncached tail.
    pub fn freeze_system(&self, original: &Value) -> Value {
        let mut blocks = vec![text_block(&self.l0, false), text_block(&self.l1, true)];
        if !self.l2.is_empty() {
            blocks.push(text_block(&self.l2, false));
        }
        if !self.l3.is_empty() {
            blocks.push(text_block(&self.l3, false));
        }
        match original {
            Value::Null => {}
            Value::String(s) if s.is_empty() => {}
            Value::String(s) => blocks.push(text_block(&canonicalize_text(s), false)),
            Value::Array(arr) => {
                for v in arr {
                    let mut c = canonicalize_json(v);
                    if let Some(o) = c.as_object_mut() {
                        o.remove("cache_control");
                    }
                    blocks.push(c);
                }
            }
            other => blocks.push(canonicalize_json(other)),
        }
        Value::Array(blocks)
    }
}

/// Live epoch + overlay prefix for `ctx inspect` / `ctx_inspect`.
pub fn render_live(store: &ctx_store::Store, session: Option<&str>) -> String {
    let Some(sid) = session.filter(|s| !s.is_empty()) else {
        return match store.epoch_count() {
            Ok(n) if n > 0 => format!("epochs {n}\n"),
            _ => String::new(),
        };
    };
    let Ok(Some(ep)) = store.current_epoch(sid) else {
        return String::new();
    };
    let overlays = store.overlays_for(sid, ep.epoch).unwrap_or_default();
    let journal = store.journal_text(sid, ep.epoch).unwrap_or_default();
    let spine = Spine::from_epoch(&ep, &overlays, &journal, "", "");
    format!(
        "Epoch {}  model {}  BASE {}\n{}",
        ep.epoch,
        if ep.model.is_empty() { "—" } else { &ep.model },
        if ep.workspace_snapshot.is_empty() {
            "—"
        } else {
            &ep.workspace_snapshot
        },
        spine.prefix()
    )
}

fn text_block(text: &str, breakpoint: bool) -> Value {
    let mut v = json!({ "type": "text", "text": text });
    if breakpoint {
        v["cache_control"] = json!({ "type": "ephemeral" });
    }
    v
}

fn render_snapshot(snapshot_id: &str, overlays: &[OverlayRow]) -> String {
    let mut out = format!("BASE {snapshot_id}\n");
    if overlays.is_empty() {
        out.push_str("OVERLAY none\n");
        return out;
    }
    out.push_str("OVERLAY\n");
    for row in overlays.iter().take(24) {
        out.push_str(&format!(
            "  {}  {} → {}\n",
            row.path,
            short(&row.prev_hash),
            short(&row.new_hash)
        ));
    }
    if overlays.len() > 24 {
        out.push_str(&format!("  … {} more\n", overlays.len() - 24));
    }
    out
}

fn short(h: &str) -> &str {
    if h.len() >= 8 {
        &h[..8]
    } else {
        h
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefix_stable_when_tail_moves() {
        let a = Spine::assemble("snap", &[], "j1", "ws1", "turn1");
        let b = Spine::assemble("snap", &[], "j2", "ws2", "turn2");
        assert_eq!(a.prefix_hash, b.prefix_hash);
        assert_ne!(a.render(), b.render());
    }

    #[test]
    fn freeze_system_puts_breakpoint_on_l1() {
        let spine = Spine::assemble("snap", &[], "journal-line", "", "");
        let sys = spine.freeze_system(&json!("be helpful"));
        let arr = sys.as_array().unwrap();
        assert!(arr[0]["text"].as_str().unwrap().contains("CTX protocol"));
        assert_eq!(arr[1]["cache_control"]["type"], "ephemeral");
        assert!(arr.iter().any(|b| b["text"].as_str() == Some("journal-line")));
        assert!(arr.iter().any(|b| b["text"].as_str() == Some("be helpful")));
    }
}
