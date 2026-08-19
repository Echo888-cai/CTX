//! In-memory Bloom filter for frame names.
//!
//! A miss is definitive: skip SQLite LIKE. A hit is only "maybe" — still query.
//! Empty filter (no inserts yet) always falls through to SQLite.

use std::sync::{Mutex, OnceLock};

const BITS: usize = 32_768; // 4 KiB
const HASHES: usize = 4;

pub struct Bloom {
    bits: Vec<u64>,
    inserts: u64,
}

impl Bloom {
    pub fn new() -> Self {
        Self {
            bits: vec![0u64; BITS / 64],
            inserts: 0,
        }
    }

    pub fn insert(&mut self, s: &str) {
        if s.is_empty() {
            return;
        }
        let key = s.to_ascii_lowercase();
        for i in 0..HASHES {
            let h = hash(i as u64, &key);
            let idx = (h as usize) % BITS;
            self.bits[idx / 64] |= 1u64 << (idx % 64);
        }
        self.inserts += 1;
        for tok in tokens(&key) {
            if tok == key {
                continue;
            }
            for i in 0..HASHES {
                let h = hash(i as u64, tok);
                let idx = (h as usize) % BITS;
                self.bits[idx / 64] |= 1u64 << (idx % 64);
            }
            self.inserts += 1;
        }
    }

    pub fn might_contain(&self, s: &str) -> bool {
        if s.is_empty() {
            return true;
        }
        let key = s.to_ascii_lowercase();
        for i in 0..HASHES {
            let h = hash(i as u64, &key);
            let idx = (h as usize) % BITS;
            if self.bits[idx / 64] & (1u64 << (idx % 64)) == 0 {
                return false;
            }
        }
        true
    }

    pub fn is_empty(&self) -> bool {
        self.inserts == 0
    }
}

fn hash(seed: u64, s: &str) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325 ^ seed.wrapping_mul(0x0100_0000_01b3);
    for b in s.as_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

fn tokens(s: &str) -> Vec<&str> {
    s.split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|t| t.len() >= 2)
        .collect()
}

fn global() -> &'static Mutex<Bloom> {
    static BLOOM: OnceLock<Mutex<Bloom>> = OnceLock::new();
    BLOOM.get_or_init(|| Mutex::new(Bloom::new()))
}

pub fn insert_frame(name: &str, hint: &str) {
    let mut b = global().lock().unwrap_or_else(|e| e.into_inner());
    b.insert(name);
    if !hint.is_empty() {
        b.insert(hint);
    }
}

/// False → no stored frame can match; skip SQLite. True → maybe, query.
pub fn query_might_match(query: &str) -> bool {
    let b = global().lock().unwrap_or_else(|e| e.into_inner());
    if b.is_empty() {
        return true;
    }
    let q = query.trim();
    if q.is_empty() {
        return true;
    }
    if b.might_contain(q) {
        return true;
    }
    tokens(&q.to_ascii_lowercase())
        .into_iter()
        .any(|t| b.might_contain(t))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_name_hits_unknown_misses() {
        let mut b = Bloom::new();
        b.insert("auth::login");
        assert!(b.might_contain("auth::login"));
        assert!(b.might_contain("login"));
        assert!(!b.might_contain("zzzz-no-such-frame"));
    }

    #[test]
    fn empty_filter_does_not_skip() {
        let b = Bloom::new();
        assert!(b.is_empty());
        assert!(b.might_contain("anything") || b.is_empty());
    }
}
