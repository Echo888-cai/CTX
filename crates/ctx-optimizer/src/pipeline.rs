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
    /// Diagnostic/error lines still present after reduction.
    pub signal_kept: u32,
}

impl OptimizeOutput {
    pub fn reduced(optimizer: &'static str, text: String) -> Self {
        let delivered_tokens = crate::tokens::estimate_tokens(&text);
        let signal_kept = crate::budget::count_signal_lines(&text);
        Self {
            text,
            optimizer,
            delivered_tokens,
            terminal: false,
            duplicate_of: None,
            signal_kept,
        }
    }

    /// Specialized guards (shell / file / mcp) must win over Generic.
    pub fn reduced_terminal(optimizer: &'static str, text: String) -> Self {
        let mut out = Self::reduced(optimizer, text);
        out.terminal = true;
        out
    }

    pub fn score(&self, signal_weight: u32) -> i64 {
        self.signal_kept as i64 * signal_weight as i64 - self.delivered_tokens as i64
    }
}

pub fn prefers(a: &OptimizeOutput, b: &OptimizeOutput, signal_weight: u32) -> bool {
    a.score(signal_weight) > b.score(signal_weight)
}

pub trait Optimizer: Send + Sync {
    fn apply(&self, input: &OptimizeInput<'_>) -> Option<OptimizeOutput>;
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(untagged)]
pub enum OptimizerSpec {
    Name(String),
    Plugin { name: String, path: String },
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

    pub fn from_names(names: &[String]) -> Self {
        let specs: Vec<OptimizerSpec> = names.iter().cloned().map(OptimizerSpec::Name).collect();
        Self::from_specs(&specs)
    }

    pub fn from_specs(specs: &[OptimizerSpec]) -> Self {
        if specs.is_empty() {
            return Self::v0();
        }
        let mut builtins: Vec<Box<dyn Optimizer>> = Vec::new();
        let mut plugins: Vec<Box<dyn Optimizer>> = Vec::new();
        for spec in specs {
            match spec {
                OptimizerSpec::Name(name) => {
                    let n = name.trim();
                    if n.is_empty() {
                        continue;
                    }
                    if looks_like_plugin_path(n) {
                        plugins.push(Box::new(crate::PluginGuard::new("plugin", n)));
                        continue;
                    }
                    match n {
                        "shell" => builtins.push(Box::new(crate::ShellGuard)),
                        "file" | "file-read" | "read" => builtins.push(Box::new(crate::ReadGuard)),
                        "mcp" => builtins.push(Box::new(crate::McpGuard)),
                        "generic" => builtins.push(Box::new(crate::GenericGuard)),
                        other => plugins.push(Box::new(crate::PluginGuard::new(other, other))),
                    }
                }
                OptimizerSpec::Plugin { name, path } => {
                    plugins.push(Box::new(crate::PluginGuard::new(name, path)));
                }
            }
        }
        let inner = if builtins.is_empty() && !plugins.is_empty() {
            let mut v = Self::v0().inner;
            v.extend(plugins);
            v
        } else if builtins.is_empty() {
            return Self::v0();
        } else {
            builtins.extend(plugins);
            builtins
        };
        Self { inner }
    }

    #[cfg(test)]
    fn guard_count(&self) -> usize {
        self.inner.len()
    }

    pub fn run(&self, input: &OptimizeInput<'_>) -> Option<OptimizeOutput> {
        let weight = input
            .metadata
            .get("signal_weight")
            .and_then(|v| v.as_u64())
            .unwrap_or(40) as u32;
        let mut best: Option<OptimizeOutput> = None;
        for opt in &self.inner {
            if let Some(out) = opt.apply(input) {
                if out.terminal {
                    return Some(out);
                }
                let better = best.as_ref().map(|b| prefers(&out, b, weight)).unwrap_or(true);
                if better {
                    best = Some(out);
                }
            }
        }
        best.filter(|out| out.delivered_tokens + crate::budget::MIN_GAIN_TOKENS < input.raw_tokens)
    }
}

fn looks_like_plugin_path(n: &str) -> bool {
    n.ends_with(".wasm") || n.contains('/') || n.contains('\\') || std::path::Path::new(n).exists()
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

    #[test]
    fn from_names_can_disable_generic() {
        let p = Pipeline::from_names(&["shell".into()]);
        assert_eq!(p.guard_count(), 1);
        let plugins_only = Pipeline::from_specs(&[OptimizerSpec::Plugin {
            name: "custom".into(),
            path: "custom.wasm".into(),
        }]);
        assert_eq!(plugins_only.guard_count(), 5, "plugins append to v0");
    }

    #[test]
    fn pipeline_prefers_keeping_panic_over_shorter_cut() {
        let keep = OptimizeOutput {
            text: "panicked at src/lib.rs:1\nerror: boom\n".into(),
            optimizer: "shell",
            delivered_tokens: 120,
            terminal: false,
            duplicate_of: None,
            signal_kept: 2,
        };
        let cut = OptimizeOutput {
            text: "ok\n".into(),
            optimizer: "generic",
            delivered_tokens: 50,
            terminal: false,
            duplicate_of: None,
            signal_kept: 0,
        };
        assert!(prefers(&keep, &cut, 40), "keep={} cut={}", keep.score(40), cut.score(40));
    }
}
