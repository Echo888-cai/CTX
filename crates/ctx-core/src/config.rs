use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Payloads below this estimated token count pass through unchanged.
    #[serde(default = "default_threshold")]
    pub virtualize_threshold_tokens: u32,
    /// Cursor/Claude file reads above this are routed to ctx_read / outlined.
    #[serde(default = "default_large_file")]
    pub large_file_tokens: u32,
    /// extreme | balanced | conservative
    #[serde(default = "default_strategy")]
    pub budget_strategy: String,
    /// Reserved: local embeddings. TF-IDF ranking is always on.
    #[serde(default)]
    pub enable_semantic: bool,
    /// Built-in names and/or `{ "name", "path" }` plugins. Empty = default pipeline.
    #[serde(default)]
    pub optimizers: Vec<ctx_optimizer::OptimizerSpec>,
    #[serde(default)]
    pub dashboard_autostart: bool,
    #[serde(default)]
    pub auto_snapshot: bool,
    /// When a session has no model id, look up this catalog / prices.json id.
    /// Empty falls back to Cursor Grok 4.6 list price.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub default_billing_model: String,
    /// Harness ids (`Harness::as_str()`) the user turned off. Empty = all on.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub disabled_harnesses: Vec<String>,
    /// Count savings but deliver the original payload unchanged.
    #[serde(default)]
    pub shadow_mode: bool,
    /// Shadow only these harness ids; empty + `shadow_mode` = global.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shadow_harnesses: Vec<String>,
    /// SimHash Hamming radius for near-duplicate tool output. 0 disables
    /// (default). Status-code / digit-run changes never collapse even if >0.
    #[serde(default = "default_hamming")]
    pub near_duplicate_hamming: u32,
}

fn default_true() -> bool {
    true
}
fn default_threshold() -> u32 {
    200
}
fn default_large_file() -> u32 {
    400
}
fn default_strategy() -> String {
    "balanced".into()
}
fn default_hamming() -> u32 {
    0
}

impl Default for Config {
    fn default() -> Self {
        Self {
            enabled: true,
            virtualize_threshold_tokens: default_threshold(),
            large_file_tokens: default_large_file(),
            budget_strategy: default_strategy(),
            enable_semantic: false,
            optimizers: Vec::new(),
            dashboard_autostart: false,
            auto_snapshot: false,
            default_billing_model: String::new(),
            disabled_harnesses: Vec::new(),
            shadow_mode: false,
            shadow_harnesses: Vec::new(),
            near_duplicate_hamming: default_hamming(),
        }
    }
}

impl Config {
    pub fn load(paths: &ctx_store::CtxPaths) -> Self {
        let p = paths.config_path();
        let Ok(bytes) = std::fs::read(&p) else {
            return Self::default();
        };
        serde_json::from_slice(&bytes).unwrap_or_default()
    }

    pub fn save(&self, paths: &ctx_store::CtxPaths) -> std::io::Result<()> {
        paths.ensure().ok();
        let json = serde_json::to_vec_pretty(self).unwrap_or_else(|_| b"{}".to_vec());
        std::fs::write(paths.config_path(), json)
    }

    pub fn is_harness_disabled(&self, harness: ctx_protocol::Harness) -> bool {
        let id = harness.as_str();
        self.disabled_harnesses.iter().any(|item| {
            item == id || (id == "claude-code" && item == "claude")
        })
    }

    pub fn is_shadow(&self, harness: ctx_protocol::Harness) -> bool {
        if self.shadow_mode && self.shadow_harnesses.is_empty() {
            return true;
        }
        let id = harness.as_str();
        self.shadow_harnesses
            .iter()
            .any(|item| item == id || (id == "claude-code" && item == "claude"))
    }
}
