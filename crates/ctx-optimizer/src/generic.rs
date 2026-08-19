use crate::ansi::strip_ansi;
use crate::budget;
use crate::pipeline::{OptimizeInput, OptimizeOutput, Optimizer};

pub struct GenericGuard;

impl Optimizer for GenericGuard {
    fn apply(&self, input: &OptimizeInput<'_>) -> Option<OptimizeOutput> {
        if input.raw_tokens < 1_200 {
            return None;
        }
        let task = task_tokens(input.metadata);
        let cap = budget::cap_hint(input.kind, input.metadata, input.payload, input.raw_tokens);
        let text =
            crate::compact::diagnostic_ranked(&crate::ansi::strip_ansi(input.payload), &task, cap);
        let out = OptimizeOutput::reduced("generic", text);
        if out.delivered_tokens + 120 >= input.raw_tokens {
            return None;
        }
        Some(out)
    }
}

fn task_tokens(metadata: &serde_json::Value) -> Vec<String> {
    metadata
        .get("task")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .split_whitespace()
        .filter(|t| t.len() >= 2)
        .map(|t| t.to_string())
        .collect()
}

pub fn reduce_text(payload: &str) -> String {
    reduce_text_for(payload, &[], 4_000)
}

pub fn reduce_text_for(payload: &str, task: &[String], raw_tokens: u32) -> String {
    crate::compact::diagnostic_ranked(&strip_ansi(payload), task, budget::cap(raw_tokens))
}
