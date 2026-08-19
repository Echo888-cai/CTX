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

    /// Specialized guards (shell / file / mcp) must win over Generic.
    pub fn reduced_terminal(optimizer: &'static str, text: String) -> Self {
        let mut out = Self::reduced(optimizer, text);
        out.terminal = true;
        out
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tokens::estimate_tokens;

    #[test]
    fn shell_wins_over_generic_on_cargo_fail() {
        let mut payload = String::from("running 200 tests\n");
        for i in 0..280 {
            payload.push_str(&format!("test t{i} ... ok\n"));
        }
        payload.push_str(
            "test auth::login ... FAILED\n\nfailures:\n\n---- auth::login stdout ----\nleft: 401\nright: 200\nredirect_uri mismatch\ntest result: FAILED. 200 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out\n",
        );
        let raw = estimate_tokens(&payload);
        assert!(raw >= 1_200, "need generic to also qualify, got {raw}");
        let input = OptimizeInput {
            kind: "shell",
            tool_name: Some("Bash"),
            payload: &payload,
            metadata: &serde_json::json!({"command": "cargo test"}),
            raw_tokens: raw,
        };
        let out = Pipeline::v0().run(&input).expect("reduce");
        assert_eq!(out.optimizer, "shell", "{}", out.text);
        assert!(out.text.contains("401"), "{}", out.text);
        assert!(out.text.contains("auth::login"), "{}", out.text);
        assert!(out.terminal);
    }
}
