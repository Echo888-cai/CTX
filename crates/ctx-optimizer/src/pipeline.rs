#[derive(Debug, Clone)]
pub struct OptimizeInput<'a> {
    pub kind: &'a str,
    pub tool_name: Option<&'a str>,
    pub payload: &'a str,
    pub metadata: &'a serde_json::Value,
    pub raw_tokens: u32,
}

#[derive(Debug, Clone)]
pub struct OptimizeOutput {
    pub text: String,
    pub optimizer: &'static str,
    pub delivered_tokens: u32,
    /// If true, caller should skip further optimizers.
    pub terminal: bool,
    pub duplicate_of: Option<String>,
}

impl OptimizeOutput {
    pub fn reduced(optimizer: &'static str, text: String) -> Self {
        let delivered_tokens = crate::tokens::estimate_tokens(&text);
        Self {
            text,
            optimizer,
            delivered_tokens,
            terminal: false,
            duplicate_of: None,
        }
    }
}

pub trait Optimizer: Send + Sync {
    fn apply(&self, input: &OptimizeInput<'_>) -> Option<OptimizeOutput>;
}

pub struct Pipeline {
    inner: Vec<Box<dyn Optimizer>>,
}

impl Pipeline {
    pub fn v0() -> Self {
        Self {
            inner: vec![
                Box::new(crate::ShellGuard),
                Box::new(crate::ReadGuard),
                Box::new(crate::McpGuard),
                Box::new(crate::GenericGuard),
            ],
        }
    }

    pub fn run(&self, input: &OptimizeInput<'_>) -> Option<OptimizeOutput> {
        let mut best: Option<OptimizeOutput> = None;
        for opt in &self.inner {
            if let Some(out) = opt.apply(input) {
                if out.terminal {
                    return Some(out);
                }
                let better = best
                    .as_ref()
                    .map(|b| out.delivered_tokens < b.delivered_tokens)
                    .unwrap_or(true);
                if better {
                    best = Some(out);
                }
            }
        }
        best.filter(|out| out.delivered_tokens + 40 < input.raw_tokens)
    }
}
