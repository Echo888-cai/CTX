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
}

fn default_true() -> bool {
    true
}
fn default_threshold() -> u32 {
    200
}
fn default_large_file() -> u32 {
    1_200
}

impl Default for Config {
    fn default() -> Self {
        Self {
            enabled: true,
            virtualize_threshold_tokens: default_threshold(),
            large_file_tokens: default_large_file(),
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
}
