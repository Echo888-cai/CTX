//! Process-local latency samples plus Prometheus text for `/metrics`.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use super::{Result, Store};

const SAMPLE_CAP: usize = 128;

struct Samples {
    ns: Mutex<Vec<u64>>,
    n: AtomicU64,
}

impl Samples {
    fn new() -> Self {
        Self {
            ns: Mutex::new(Vec::with_capacity(SAMPLE_CAP)),
            n: AtomicU64::new(0),
        }
    }

    fn record(&self, d: Duration) {
        let v = d.as_nanos().min(u128::from(u64::MAX)) as u64;
        self.n.fetch_add(1, Ordering::Relaxed);
        let mut g = self.ns.lock().unwrap_or_else(|e| e.into_inner());
        if g.len() == SAMPLE_CAP {
            g.remove(0);
        }
        g.push(v);
    }

    fn quantile(&self, q: f64) -> f64 {
        let g = self.ns.lock().unwrap_or_else(|e| e.into_inner());
        if g.is_empty() {
            return 0.0;
        }
        let mut s = g.clone();
        s.sort_unstable();
        let idx = ((q * (s.len() as f64 - 1.0)).round() as usize).min(s.len() - 1);
        s[idx] as f64 / 1_000_000_000.0
    }

    fn count(&self) -> u64 {
        self.n.load(Ordering::Relaxed)
    }
}

fn page_faults() -> &'static Samples {
    static S: OnceLock<Samples> = OnceLock::new();
    S.get_or_init(Samples::new)
}

fn hooks() -> &'static Samples {
    static S: OnceLock<Samples> = OnceLock::new();
    S.get_or_init(Samples::new)
}

pub fn record_page_fault(d: Duration) {
    page_faults().record(d);
}

pub fn record_hook(d: Duration) {
    hooks().record(d);
}

impl Store {
    pub fn prometheus_text(&self) -> Result<String> {
        let pages = self.page_count()?;
        let blobs = self.blob_count()?;
        let bytes = self.compressed_bytes()?;
        let obs = self.observation_count()?;
        let (hits, misses) = crate::cache::stats();
        let denom = hits.saturating_add(misses);
        let ratio = if denom == 0 {
            0.0
        } else {
            hits as f64 / denom as f64
        };
        let avoided = self.avoided_by_optimizer()?;
        let mut out = String::new();
        out.push_str("# HELP ctx_observations_total Stored tool-output observations\n");
        out.push_str("# TYPE ctx_observations_total counter\n");
        out.push_str(&format!("ctx_observations_total {obs}\n"));
        out.push_str("# HELP ctx_optimizer_avoided_tokens Tokens kept out of the model\n");
        out.push_str("# TYPE ctx_optimizer_avoided_tokens counter\n");
        if avoided.is_empty() {
            out.push_str("ctx_optimizer_avoided_tokens{optimizer=\"none\"} 0\n");
        } else {
            for (name, n) in avoided {
                out.push_str(&format!(
                    "ctx_optimizer_avoided_tokens{{optimizer=\"{}\"}} {n}\n",
                    prom_label(&name)
                ));
            }
        }
        out.push_str("# HELP ctx_page_fault_latency_seconds Page-in latency\n");
        out.push_str("# TYPE ctx_page_fault_latency_seconds summary\n");
        let pf = page_faults();
        out.push_str(&format!(
            "ctx_page_fault_latency_seconds{{quantile=\"0.5\"}} {}\n",
            pf.quantile(0.5)
        ));
        out.push_str(&format!(
            "ctx_page_fault_latency_seconds{{quantile=\"0.9\"}} {}\n",
            pf.quantile(0.9)
        ));
        out.push_str(&format!(
            "ctx_page_fault_latency_seconds{{quantile=\"0.99\"}} {}\n",
            pf.quantile(0.99)
        ));
        out.push_str(&format!(
            "ctx_page_fault_latency_seconds_count {}\n",
            pf.count()
        ));
        out.push_str("# HELP ctx_hook_latency_seconds Hook handler latency\n");
        out.push_str("# TYPE ctx_hook_latency_seconds summary\n");
        let hk = hooks();
        out.push_str(&format!(
            "ctx_hook_latency_seconds{{quantile=\"0.5\"}} {}\n",
            hk.quantile(0.5)
        ));
        out.push_str(&format!(
            "ctx_hook_latency_seconds{{quantile=\"0.9\"}} {}\n",
            hk.quantile(0.9)
        ));
        out.push_str(&format!(
            "ctx_hook_latency_seconds{{quantile=\"0.99\"}} {}\n",
            hk.quantile(0.99)
        ));
        out.push_str(&format!("ctx_hook_latency_seconds_count {}\n", hk.count()));
        out.push_str("# HELP ctx_cache_hit_ratio ARC decompressed-page hits\n");
        out.push_str("# TYPE ctx_cache_hit_ratio gauge\n");
        out.push_str(&format!("ctx_cache_hit_ratio {ratio:.6}\n"));
        out.push_str("# HELP ctx_cache_hits_total ARC cache hits\n");
        out.push_str("# TYPE ctx_cache_hits_total counter\n");
        out.push_str(&format!("ctx_cache_hits_total {hits}\n"));
        out.push_str("# HELP ctx_store_compressed_bytes Blob bytes on disk\n");
        out.push_str("# TYPE ctx_store_compressed_bytes gauge\n");
        out.push_str(&format!("ctx_store_compressed_bytes {bytes}\n"));
        out.push_str("# HELP ctx_store_blob_count Content-addressed blobs\n");
        out.push_str("# TYPE ctx_store_blob_count gauge\n");
        out.push_str(&format!("ctx_store_blob_count {blobs}\n"));
        out.push_str("# HELP ctx_store_page_count Indexed pages\n");
        out.push_str("# TYPE ctx_store_page_count gauge\n");
        out.push_str(&format!("ctx_store_page_count {pages}\n"));
        Ok(out)
    }
}

fn prom_label(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quantile_empty_is_zero() {
        let s = Samples::new();
        assert_eq!(s.quantile(0.5), 0.0);
    }

    #[test]
    fn quantile_tracks_samples() {
        let s = Samples::new();
        s.record(Duration::from_millis(1));
        s.record(Duration::from_millis(10));
        s.record(Duration::from_millis(100));
        assert!(s.quantile(0.5) > 0.0);
        assert!(s.quantile(0.99) >= s.quantile(0.5));
    }
}
