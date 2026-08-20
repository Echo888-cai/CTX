//! Content-defined chunking (FastCDC-style) for incremental file reads.

use serde::{Deserialize, Serialize};

const MIN: usize = 512;
const AVG: usize = 2048;
const MAX: usize = 8192;
const MASK: u64 = (1 << 11) - 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Chunk {
    pub offset: usize,
    pub len: usize,
    pub hash: String,
}

/// Gear table: 256 mixing constants. Deterministic, no extra crate.
fn gear(b: u8) -> u64 {
    const GOLDEN: u64 = 0x9e3779b97f4a7c15;
    (b as u64).wrapping_add(1).wrapping_mul(GOLDEN)
}

pub fn chunk_text(text: &str) -> Vec<Chunk> {
    chunk_bytes(text.as_bytes())
}

pub fn chunk_bytes(data: &[u8]) -> Vec<Chunk> {
    if data.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut start = 0usize;
    let mut hash = 0u64;
    for i in 0..data.len() {
        hash = hash.wrapping_shl(1).wrapping_add(gear(data[i]));
        let size = i - start + 1;
        let boundary = size >= MAX
            || (size >= MIN && (hash & MASK) == 0)
            || (size >= AVG && (hash & (MASK >> 1)) == 0);
        if boundary || i + 1 == data.len() {
            let slice = &data[start..=i];
            out.push(Chunk {
                offset: start,
                len: slice.len(),
                hash: ctx_protocol_hash(slice),
            });
            start = i + 1;
            hash = 0;
        }
    }
    out
}

fn ctx_protocol_hash(slice: &[u8]) -> String {
    // Local FNV so optimizer stays free of blake3.
    let mut h = 0xcbf29ce484222325u64;
    for b in slice {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}")
}

/// Keep unchanged chunks as a URI handle; emit changed slices as text.
pub fn cdc_working_set(prev: &[Chunk], curr: &[Chunk], text: &str, uri: &str) -> Option<String> {
    if prev.is_empty() || curr.is_empty() || prev == curr {
        return None;
    }
    let prev_hashes: std::collections::HashSet<&str> = prev.iter().map(|c| c.hash.as_str()).collect();
    let mut kept = 0u32;
    let mut changed = String::new();
    for c in curr {
        if prev_hashes.contains(c.hash.as_str()) {
            kept += 1;
            continue;
        }
        let end = (c.offset + c.len).min(text.len());
        if c.offset < text.len() {
            changed.push_str(&text[c.offset..end]);
            if !changed.ends_with('\n') {
                changed.push('\n');
            }
        }
    }
    if changed.is_empty() {
        return None;
    }
    Some(format!(
        "file Δ {}/{} chunks  {uri} keeps {kept}\n---\n{changed}",
        curr.len() as u32 - kept,
        curr.len(),
        uri = uri,
        kept = kept,
        changed = changed
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_edit_reuses_most_chunks() {
        let a = "hello world\n".repeat(900);
        let mut b = a.clone();
        b.replace_range(20..25, "HELLO");
        let ca = chunk_text(&a);
        let cb = chunk_text(&b);
        assert!(ca.len() >= 2, "{}", ca.len());
        let shared = ca.iter().filter(|x| cb.iter().any(|y| y.hash == x.hash)).count();
        assert!(shared + 2 >= ca.len().min(cb.len()), "shared={shared} a={} b={}", ca.len(), cb.len());
        let out = cdc_working_set(&ca, &cb, &b, "ctx://file/x").expect("delta");
        assert!(out.contains("keeps"), "{out}");
        assert!(out.len() < a.len(), "{} vs {}", out.len(), a.len());
    }
}
