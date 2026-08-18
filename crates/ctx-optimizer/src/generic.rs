use crate::ansi::strip_ansi;
use crate::pipeline::{OptimizeInput, OptimizeOutput, Optimizer};

pub struct GenericGuard;

impl Optimizer for GenericGuard {
    fn apply(&self, input: &OptimizeInput<'_>) -> Option<OptimizeOutput> {
        if input.raw_tokens < 1_200 {
            return None;
        }
        let text = reduce_text(input.payload);
        let out = OptimizeOutput::reduced("generic", text);
        if out.delivered_tokens + 120 >= input.raw_tokens {
            return None;
        }
        Some(out)
    }
}

pub fn reduce_text(payload: &str) -> String {
    crate::compact::diagnostic_preview(&strip_ansi(payload), 50, 40)
}
