//! Semantic working-set ranking. Zero extra model tokens.
//!
//! TF-IDF is the default ranker. An optional embedding cache can implement
//! [`SemanticRanker`] later without changing [`select_mapped`](super::WorkingSet).

use std::collections::{HashMap, HashSet};

use ctx_store::PageMeta;

use crate::task::{overlap, parse_task, token_matches};

/// Rank pages for a task query. Higher score is a better map.
pub trait SemanticRanker: Send + Sync {
    fn rank(&self, query: &[String], pages: &[PageMeta]) -> Vec<(usize, f32)>;
}

/// Classic TF-IDF over deterministic task tokens. No model, no extra deps.
pub struct TfIdfRanker;

impl SemanticRanker for TfIdfRanker {
    fn rank(&self, query: &[String], pages: &[PageMeta]) -> Vec<(usize, f32)> {
        if query.is_empty() || pages.is_empty() {
            return Vec::new();
        }
        let docs: Vec<Vec<String>> = pages.iter().map(|p| parse_task(&p.task)).collect();
        let n = docs.len() as f32;
        let mut df: HashMap<String, u32> = HashMap::new();
        for doc in &docs {
            let unique: HashSet<&str> = doc.iter().map(String::as_str).collect();
            for t in unique {
                *df.entry(t.to_string()).or_default() += 1;
            }
        }
        let mut scored: Vec<(usize, f32)> = Vec::with_capacity(docs.len());
        for (i, doc) in docs.iter().enumerate() {
            let mut tf: HashMap<&str, u32> = HashMap::new();
            for t in doc {
                *tf.entry(t.as_str()).or_default() += 1;
            }
            let len = doc.len().max(1) as f32;
            let mut score = 0.0f32;
            for q in query {
                let mut hits = 0u32;
                for (term, count) in &tf {
                    if token_matches(term, q) {
                        hits = hits.saturating_add(*count);
                    }
                }
                if hits == 0 {
                    continue;
                }
                let d = df_for(&df, q, doc);
                let idf = ((n + 1.0) / (d as f32 + 1.0)).ln().max(0.0);
                score += (hits as f32 / len) * idf * overlap_boost(q);
            }
            scored.push((i, score));
        }
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored
    }
}

fn df_for(df: &HashMap<String, u32>, query: &str, doc: &[String]) -> u32 {
    if let Some(n) = df.get(query) {
        return *n;
    }
    // Substring hits (oauth ⊂ oauth2) still need an IDF so rare terms win.
    df.iter()
        .filter(|(term, _)| token_matches(term, query))
        .map(|(_, n)| *n)
        .max()
        .unwrap_or_else(|| {
            if overlap(doc, std::slice::from_ref(&query.to_string())) > 0 {
                1
            } else {
                0
            }
        })
}

fn overlap_boost(q: &str) -> f32 {
    match q.chars().count() {
        0..=2 => 0.6,
        3..=5 => 1.0,
        _ => 1.3,
    }
}

/// Score vector aligned with `pages` (0.0 if absent).
pub fn tfidf_scores(query: &[String], pages: &[PageMeta]) -> Vec<f32> {
    let mut out = vec![0.0; pages.len()];
    for (i, s) in TfIdfRanker.rank(query, pages) {
        if i < out.len() {
            out[i] = s;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(uri: &str, task: &str) -> PageMeta {
        PageMeta {
            uri: uri.into(),
            hash: uri.into(),
            kind: "shell".into(),
            summary: None,
            raw_tokens: 40,
            created_at: 1,
            task: task.into(),
            harness: "claude-code".into(),
        }
    }

    #[test]
    fn rare_term_outranks_common_term() {
        let mut pages = Vec::new();
        for i in 0..8 {
            pages.push(page(&format!("ctx://shell/bill{i}"), "billing invoice"));
        }
        pages.push(page("ctx://file/oauth1", "oauth redirect"));
        let query = crate::extract_task(&["oauth billing"]);
        let ranked = TfIdfRanker.rank(&query, &pages);
        assert_eq!(pages[ranked[0].0].uri, "ctx://file/oauth1", "{ranked:?}");
        assert!(ranked[0].1 > ranked[1].1, "{ranked:?}");
    }

    #[test]
    fn empty_query_is_empty() {
        let pages = vec![page("ctx://shell/a", "auth login")];
        assert!(TfIdfRanker.rank(&[], &pages).is_empty());
    }
}
