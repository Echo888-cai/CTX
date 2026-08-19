//! Adaptive Replacement Cache for decompressed pages.
//!
//! Balances recency (T1) and frequency (T2) with ghost lists (B1/B2) so a
//! one-shot scan does not evict a page the model is about to fetch.

use std::collections::{HashMap, VecDeque};
use std::hash::Hash;
use std::sync::{Arc, Mutex, OnceLock};

const DEFAULT_CAP: usize = 32;
const MAX_PAGE_BYTES: usize = 2 * 1024 * 1024;

pub struct ArcCache<K, V> {
    t1: VecDeque<K>,
    t2: VecDeque<K>,
    b1: VecDeque<K>,
    b2: VecDeque<K>,
    map: HashMap<K, V>,
    p: usize,
    cap: usize,
}

impl<K: Clone + Eq + Hash, V> ArcCache<K, V> {
    pub fn new(cap: usize) -> Self {
        Self {
            t1: VecDeque::new(),
            t2: VecDeque::new(),
            b1: VecDeque::new(),
            b2: VecDeque::new(),
            map: HashMap::new(),
            p: 0,
            cap: cap.max(1),
        }
    }

    pub fn get(&mut self, key: &K) -> Option<&V> {
        if self.t1.iter().any(|k| k == key) {
            Self::remove_from(&mut self.t1, key);
            self.t2.push_back(key.clone());
            return self.map.get(key);
        }
        if self.t2.iter().any(|k| k == key) {
            Self::remove_from(&mut self.t2, key);
            self.t2.push_back(key.clone());
            return self.map.get(key);
        }
        None
    }

    pub fn insert(&mut self, key: K, value: V) {
        if self.map.contains_key(&key) {
            self.map.insert(key.clone(), value);
            let _ = self.get(&key);
            return;
        }
        let in_b1 = self.b1.iter().any(|k| k == &key);
        let in_b2 = self.b2.iter().any(|k| k == &key);
        if in_b1 {
            self.p = (self.p + 1).min(self.cap);
            Self::remove_from(&mut self.b1, &key);
            self.replace(false);
            self.t2.push_back(key.clone());
            self.map.insert(key, value);
            return;
        }
        if in_b2 {
            self.p = self.p.saturating_sub(1);
            Self::remove_from(&mut self.b2, &key);
            self.replace(true);
            self.t2.push_back(key.clone());
            self.map.insert(key, value);
            return;
        }
        let l1 = self.t1.len() + self.b1.len();
        if l1 == self.cap {
            if self.t1.len() < self.cap {
                self.b1.pop_front();
                self.replace(false);
            } else {
                if let Some(old) = self.t1.pop_front() {
                    self.map.remove(&old);
                }
            }
        } else if l1 + self.t2.len() + self.b2.len() >= self.cap {
            if l1 + self.t2.len() + self.b2.len() >= 2 * self.cap {
                self.b2.pop_front();
            }
            self.replace(false);
        }
        self.t1.push_back(key.clone());
        self.map.insert(key, value);
    }

    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.map.len()
    }

    fn replace(&mut self, in_b2: bool) {
        if !self.t1.is_empty() && (self.t1.len() > self.p || (in_b2 && self.t1.len() == self.p)) {
            if let Some(old) = self.t1.pop_front() {
                self.map.remove(&old);
                self.b1.push_back(old);
            }
        } else if let Some(old) = self.t2.pop_front() {
            self.map.remove(&old);
            self.b2.push_back(old);
        }
    }

    fn remove_from(q: &mut VecDeque<K>, key: &K) {
        if let Some(i) = q.iter().position(|k| k == key) {
            q.remove(i);
        }
    }
}

struct GlobalCache {
    inner: Mutex<ArcCache<String, Arc<Vec<u8>>>>,
    hits: std::sync::atomic::AtomicU64,
    misses: std::sync::atomic::AtomicU64,
}

fn global() -> &'static GlobalCache {
    static CACHE: OnceLock<GlobalCache> = OnceLock::new();
    CACHE.get_or_init(|| GlobalCache {
        inner: Mutex::new(ArcCache::new(DEFAULT_CAP)),
        hits: std::sync::atomic::AtomicU64::new(0),
        misses: std::sync::atomic::AtomicU64::new(0),
    })
}

pub fn get(hash: &str) -> Option<Arc<Vec<u8>>> {
    let g = global();
    let mut inner = g.inner.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(v) = inner.get(&hash.to_string()) {
        g.hits.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        return Some(Arc::clone(v));
    }
    g.misses.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    None
}

pub fn insert(hash: String, bytes: Arc<Vec<u8>>) {
    if bytes.len() > MAX_PAGE_BYTES {
        return;
    }
    let g = global();
    let mut inner = g.inner.lock().unwrap_or_else(|e| e.into_inner());
    inner.insert(hash, bytes);
}

pub fn stats() -> (u64, u64) {
    let g = global();
    (
        g.hits.load(std::sync::atomic::Ordering::Relaxed),
        g.misses.load(std::sync::atomic::Ordering::Relaxed),
    )
}

/// Decode an immutable content-addressed blob. Prefers mmap so the compressed
/// bytes are not copied into a second heap buffer before zstd.
pub fn decode_blob_file(path: &std::path::Path) -> std::io::Result<Vec<u8>> {
    let file = std::fs::File::open(path)?;
    let len = file.metadata()?.len();
    if len == 0 {
        return Ok(Vec::new());
    }
    // SAFETY: blobs are content-addressed and never overwritten in place
    // (writes go to a .tmp then rename). The mapping is dropped after decode.
    match unsafe { memmap2::Mmap::map(&file) } {
        Ok(mmap) => zstd::decode_all(&mmap[..]),
        Err(_) => {
            let compressed = std::fs::read(path)?;
            zstd::decode_all(compressed.as_slice())
        }
    }
}

/// Decompress listed blobs into the ARC cache. Best-effort; never errors.
pub fn prefetch_blobs(paths: &crate::CtxPaths, hashes: &[String]) {
    let mut n = 0u32;
    for hash in hashes {
        if hash.is_empty() || get(hash).is_some() {
            continue;
        }
        let dest = paths.store_dir().join(crate::blob::blob_relpath(hash));
        let Ok(raw) = decode_blob_file(&dest) else {
            continue;
        };
        insert(hash.clone(), Arc::new(raw));
        n += 1;
    }
    let _ = n;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frequent_key_survives_scan() {
        let mut c = ArcCache::new(4);
        c.insert(0usize, 1);
        assert_eq!(c.get(&0), Some(&1));
        for i in 1..9usize {
            c.insert(i, i);
        }
        assert_eq!(c.get(&0), Some(&1), "ARC should keep the frequent key");
    }

    #[test]
    fn capacity_is_respected() {
        let mut c = ArcCache::new(2);
        c.insert("a", 1);
        c.insert("b", 2);
        c.insert("c", 3);
        assert!(c.len() <= 2);
    }
}
