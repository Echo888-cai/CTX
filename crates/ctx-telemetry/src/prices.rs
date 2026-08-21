//! Local input price book. Dollars are API-equivalent estimates, not invoices.
//!
//! The bundled catalog is a snapshot of Cursor's public model list. Dashboard
//! loads also refresh `https://cursor.com/docs/models-and-pricing.md` into
//! `prices.official.json` when the cache is older than 12 hours.

use std::collections::HashMap;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use ctx_store::CtxPaths;
use serde::Deserialize;
use serde_json::Value;

const OFFICIAL_SOURCE: &str = "https://cursor.com/docs/models-and-pricing.md";
const MODELS_DEV_SOURCE: &str = "https://models.dev/api.json";
const CACHE_TTL_SECS: i64 = 12 * 3600;
const FALLBACK_MODEL: &str = "grok-4.6";

/// Public API / Cursor list input prices (USD / 1M tokens). Local snapshot, not live.
const CATALOG: &[(&str, f64, &str)] = &[
    ("grok-4.6", 2.0, "Grok 4.6"),
    ("grok-4.6-fast", 4.0, "Grok 4.6 Fast"),
    ("grok-4.5", 2.0, "Grok 4.5"),
    ("grok-4.5-fast", 4.0, "Grok 4.5 Fast"),
    ("composer-2.5", 0.50, "Composer 2.5"),
    ("composer-2.5-fast", 3.0, "Composer 2.5 Fast"),
    ("claude-opus-4", 15.0, "Claude Opus 4"),
    ("claude-sonnet-4", 3.0, "Claude Sonnet 4"),
    ("claude-haiku-4", 0.80, "Claude Haiku 4"),
    ("claude-4.6-sonnet", 3.0, "Claude 4.6 Sonnet"),
    ("claude-4.6-opus", 5.0, "Claude 4.6 Opus"),
    ("claude-sonnet-5", 2.0, "Claude Sonnet 5"),
    ("claude-opus-5", 5.0, "Claude Opus 5"),
    ("gpt-5", 1.25, "GPT-5"),
    ("gpt-5-mini", 0.25, "GPT-5 Mini"),
    ("gpt-4.1", 2.0, "GPT-4.1"),
    ("gpt-4o", 2.50, "GPT-4o"),
    ("o3", 10.0, "o3"),
    ("o4-mini", 1.10, "o4-mini"),
    ("gemini-2.5-pro", 1.25, "Gemini 2.5 Pro"),
    ("gemini-2.5-flash", 0.30, "Gemini 2.5 Flash"),
    ("deepseek-chat", 0.28, "DeepSeek Chat"),
    ("deepseek-v4-flash", 0.14, "DeepSeek V4 Flash"),
];

const ALIASES: &[(&str, &str)] = &[
    ("claude-opus-4-1", "claude-opus-4"),
    ("claude-opus-4-6", "claude-opus-4"),
    ("claude-4-opus", "claude-opus-4"),
    ("claude-opus-4-20250514", "claude-opus-4"),
    ("claude-sonnet-4-5", "claude-sonnet-4"),
    ("claude-sonnet-4-6", "claude-sonnet-4"),
    ("claude-4-sonnet", "claude-sonnet-4"),
    ("claude-sonnet-4-20250514", "claude-sonnet-4"),
    ("claude-haiku-4-5", "claude-haiku-4"),
    ("claude-3-5-haiku", "claude-haiku-4"),
    ("claude-haiku-3-5", "claude-haiku-4"),
    ("chatgpt-4o", "gpt-4o"),
    ("cursor-grok-4.6", "grok-4.6"),
    ("cursor-grok-4.6-fast", "grok-4.6-fast"),
    ("cursor-grok-4.5", "grok-4.5"),
    ("cursor-composer-2.5", "composer-2.5"),
    ("composer-2", "composer-2.5"),
];

const EFFORT_SUFFIXES: &[&str] = &[
    "-xhigh",
    "-high",
    "-medium",
    "-med",
    "-low",
    "-fast",
    "-thinking",
    "-max",
];

/// Where a rate came from. Surfaced so the dashboard never presents a guess
/// as if it were a quote.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PriceSource {
    /// Fetched from Cursor's public models-and-pricing page.
    Official,
    /// Bundled snapshot compiled into the binary.
    Catalog,
    /// Local prices.json.
    Override,
    /// The model id was Auto/unknown, so the default billing model was used.
    Fallback,
}

impl PriceSource {
    pub fn as_str(self) -> &'static str {
        match self {
            PriceSource::Official => "official",
            PriceSource::Catalog => "catalog",
            PriceSource::Override => "override",
            PriceSource::Fallback => "fallback",
        }
    }

    /// True when the number is an estimate rather than this model's list price.
    pub fn is_estimate(self) -> bool {
        matches!(self, PriceSource::Fallback)
    }
}

#[derive(Debug, Clone)]
pub struct PriceQuote {
    pub usd_per_mtok: f64,
    pub source: PriceSource,
    /// Catalog id the lookup landed on, e.g. `grok-4.6`.
    pub matched_id: String,
}

#[derive(Debug, Clone)]
pub struct PriceBook {
    prices: HashMap<String, (f64, PriceSource)>,
    extras: HashMap<String, RateExtra>,
    aliases: HashMap<String, String>,
    default_billing_model: String,
}

#[derive(Debug, Clone, Copy, Default)]
struct RateExtra {
    output: Option<f64>,
    cache_read: Option<f64>,
    cache_write: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct CatalogEntry {
    pub id: &'static str,
    pub name: &'static str,
    pub input_usd_per_mtok: f64,
}

impl PriceBook {
    pub fn load(paths: &CtxPaths, default_billing_model: &str) -> Self {
        let mut prices = HashMap::new();
        for (id, usd, _) in CATALOG {
            prices.insert((*id).to_string(), (*usd, PriceSource::Catalog));
        }
        let mut aliases = HashMap::new();
        for (from, to) in ALIASES {
            aliases.insert((*from).to_string(), (*to).to_string());
        }
        let mut extras = HashMap::new();
        merge_overrides(
            &mut prices,
            &mut extras,
            &paths.official_prices_path(),
            PriceSource::Official,
        );
        merge_overrides(
            &mut prices,
            &mut extras,
            &paths.prices_path(),
            PriceSource::Override,
        );
        let default = default_billing_model.trim();
        Self {
            prices,
            extras,
            aliases,
            default_billing_model: if default.is_empty() {
                FALLBACK_MODEL.to_string()
            } else {
                default.to_string()
            },
        }
    }

    /// Same as [`load`], but refreshes Cursor's public price list when the cache is stale.
    pub fn load_with_refresh(paths: &CtxPaths, default_billing_model: &str) -> Self {
        let _ = refresh_official_prices(paths);
        Self::load(paths, default_billing_model)
    }

    pub fn default_billing_model(&self) -> &str {
        &self.default_billing_model
    }

        pub fn catalog() -> Vec<CatalogEntry> {
        CATALOG
            .iter()
            .map(|(id, usd, name)| CatalogEntry {
                id,
                name,
                input_usd_per_mtok: *usd,
            })
            .collect()
    }

    /// Input USD per million tokens, if this id (or the default billing model) is priced.
    pub fn input_usd_per_mtok(&self, model_id: &str) -> Option<f64> {
        self.quote(model_id).map(|q| q.usd_per_mtok)
    }

    /// Rate plus provenance. Auto / unknown ids return `None` — inventing a
    /// dollar figure without a named model is not honest.
    pub fn quote(&self, model_id: &str) -> Option<PriceQuote> {
        let key = self.resolve(model_id)?;
        let (usd, source) = self.prices.get(&key).copied()?;
        Some(PriceQuote {
            usd_per_mtok: usd,
            source,
            matched_id: key,
        })
    }

    pub fn avoided_usd(&self, model_id: &str, avoided_tokens: u64) -> Option<f64> {
        let price = self.input_usd_per_mtok(model_id)?;
        Some(round_usd(avoided_tokens as f64 / 1_000_000.0 * price))
    }

    /// Stable family id for dashboard grouping.
    ///
    /// Cursor effort variants (`cursor-grok-4.5-high-fast`) and bare API ids
    /// (`grok-4.5`) collapse to the same catalog key so the model list does not
    /// show duplicate display names. Unlike [`Self::resolve`], this deliberately
    /// ignores whether a `cursor-*` key exists in the price map — official
    /// caches often price the Cursor-prefixed id, which would otherwise keep
    /// `cursor-grok-4.5` and `grok-4.5` as separate families.
    pub fn canonical_id(&self, model_id: &str) -> String {
        let id = model_id.trim();
        if id.is_empty() || id == "__unknown__" || id.eq_ignore_ascii_case("unknown") {
            return "__unknown__".into();
        }
        if id.eq_ignore_ascii_case("default") || id.eq_ignore_ascii_case("auto") {
            return "default".into();
        }
        let mut key = normalize(id);
        for _ in 0..8 {
            if let Some(canon) = self.aliases.get(&key) {
                if canon != &key {
                    key = canon.clone();
                    continue;
                }
            }
            let bare = strip_cursor_prefix(&key);
            if bare != key {
                key = bare.to_string();
                continue;
            }
            if let Some(next) = strip_effort_suffix(&key) {
                key = next;
                continue;
            }
            break;
        }
        key
    }

    /// Human label for a stored model id: catalog name when known, else the id,
    /// else `Other` / `Auto`.
    pub fn display_name(model_id: &str) -> String {
        let id = model_id.trim();
        if id.is_empty() || id == "__unknown__" || id.eq_ignore_ascii_case("unknown") {
            return "Other".into();
        }
        if id.eq_ignore_ascii_case("default") || id.eq_ignore_ascii_case("auto") {
            return "Auto".into();
        }
        let mut key = normalize(id);
        for _ in 0..8 {
            if let Some((_, _, name)) = CATALOG.iter().find(|(cid, _, _)| normalize(cid) == key) {
                return (*name).into();
            }
            if let Some((_, to)) = ALIASES.iter().find(|(from, _)| normalize(from) == key) {
                key = normalize(to);
                continue;
            }
            let bare = strip_cursor_prefix(&key);
            if bare != key {
                key = bare.to_string();
                continue;
            }
            if let Some(next) = strip_effort_suffix(&key) {
                key = next;
                continue;
            }
            break;
        }
        let normalized = normalize(id);
        let bare = strip_cursor_prefix(&normalized);
        if bare != normalized {
            if let Some(next) = strip_effort_suffix(bare) {
                return next;
            }
            return bare.to_string();
        }
        id.to_string()
    }

    fn resolve(&self, model_id: &str) -> Option<String> {
        let raw = model_id.trim();
        if is_auto_id(raw) {
            return None;
        }
        let mut key = normalize(raw);
        for _ in 0..8 {
            if self.prices.contains_key(&key) {
                return Some(key);
            }
            if let Some(canon) = self.aliases.get(&key) {
                if self.prices.contains_key(canon) {
                    return Some(canon.clone());
                }
                key = canon.clone();
                continue;
            }
            let stripped = strip_cursor_prefix(&key);
            if stripped != key && self.prices.contains_key(stripped) {
                return Some(stripped.to_string());
            }
            if let Some(next) = strip_effort_suffix(&key) {
                key = next;
                continue;
            }
            break;
        }
        None
    }
}

/// Cursor reports `default` for Auto, and sessions without a hook payload land
/// on `__unknown__`. Neither names a model, so both price as estimates.
pub fn is_auto_id(id: &str) -> bool {
    let id = id.trim();
    id.is_empty()
        || id == "__unknown__"
        || id.eq_ignore_ascii_case("unknown")
        || id.eq_ignore_ascii_case("default")
        || id.eq_ignore_ascii_case("auto")
}

pub fn catalog_json() -> Value {
    Value::Array(
        PriceBook::catalog()
            .into_iter()
            .map(|e| {
                serde_json::json!({
                    "id": e.id,
                    "name": e.name,
                    "input_usd_per_mtok": e.input_usd_per_mtok,
                    "cache_read_usd_per_mtok": crate::TierRates::for_model(e.id, e.input_usd_per_mtok).cache_read,
                    "output_usd_per_mtok": crate::TierRates::for_model(e.id, e.input_usd_per_mtok).output,
                })
            })
            .collect(),
    )
}

pub fn round_usd(n: f64) -> f64 {
    (n * 1_000_000.0).round() / 1_000_000.0
}

/// Fetch Cursor's public pricing markdown and cache input USD / 1M tokens.
pub fn refresh_official_prices(paths: &CtxPaths) -> bool {
    refresh_official_prices_inner(paths, false)
}

pub fn refresh_official_prices_now(paths: &CtxPaths) -> bool {
    refresh_official_prices_inner(paths, true)
}

pub fn official_price_meta(paths: &CtxPaths) -> (usize, i64) {
    let Ok(bytes) = std::fs::read(paths.official_prices_path()) else {
        return (PriceBook::catalog().len(), 0);
    };
    let Ok(value) = serde_json::from_slice::<Value>(&bytes) else {
        return (PriceBook::catalog().len(), 0);
    };
    let fetched = value
        .get("_meta")
        .and_then(|m| m.get("fetched_at"))
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let entries = value
        .as_object()
        .map(|o| o.keys().filter(|k| *k != "_meta").count())
        .unwrap_or(0)
        .max(PriceBook::catalog().len());
    (entries, fetched)
}

fn refresh_official_prices_inner(paths: &CtxPaths, force: bool) -> bool {
    let dest = paths.official_prices_path();
    if !force && cache_fresh(&dest) {
        return false;
    }
    let Some(body) = fetch_text(OFFICIAL_SOURCE) else {
        return false;
    };
    let parsed = parse_cursor_pricing_markdown(&body);
    if parsed.is_empty() {
        return false;
    }
    let mut obj = serde_json::Map::new();
    obj.insert(
        "_meta".into(),
        serde_json::json!({
            "source": OFFICIAL_SOURCE,
            "fetched_at": now_unix(),
        }),
    );
    for (id, usd) in parsed {
        obj.insert(id, serde_json::json!(usd));
    }
    if let Some(dev) = fetch_text(MODELS_DEV_SOURCE) {
        merge_models_dev(&mut obj, &dev);
        if let Some(meta) = obj.get_mut("_meta").and_then(|v| v.as_object_mut()) {
            meta.insert("models_dev".into(), serde_json::json!(true));
        }
    }
    let Ok(bytes) = serde_json::to_vec_pretty(&Value::Object(obj)) else {
        return false;
    };
    let _ = paths.ensure();
    std::fs::write(dest, bytes).is_ok()
}

pub fn parse_cursor_pricing_markdown(md: &str) -> HashMap<String, f64> {
    let mut out = HashMap::new();
    for line in md.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with('|') || !trimmed.contains('$') {
            continue;
        }
        let cols: Vec<&str> = trimmed.split('|').map(str::trim).collect();
        if cols.len() < 5 {
            continue;
        }
        let name = strip_md_link(cols[1]);
        if name.is_empty() || name.eq_ignore_ascii_case("model") {
            continue;
        }
        let Some(usd) = parse_usd(cols[3]) else {
            continue;
        };
        for id in ids_for_model_name(&name) {
            out.entry(id).or_insert(usd);
        }
    }
    out
}

fn cache_fresh(path: &std::path::Path) -> bool {
    let Ok(bytes) = std::fs::read(path) else {
        return false;
    };
    let Ok(value) = serde_json::from_slice::<Value>(&bytes) else {
        return false;
    };
    let fetched = value
        .get("_meta")
        .and_then(|m| m.get("fetched_at"))
        .and_then(Value::as_i64)
        .unwrap_or(0);
    fetched > 0 && now_unix().saturating_sub(fetched) < CACHE_TTL_SECS
}

fn fetch_text(url: &str) -> Option<String> {
    let output = Command::new("curl")
        .args([
            "-fsSL",
            "--max-time",
            "4",
            "-A",
            "CTX-dashboard/0.2",
            url,
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout).ok()
}

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn normalize(id: &str) -> String {
    let trimmed = id.trim().to_lowercase();
    trimmed
        .rsplit(['/', ':'])
        .next()
        .unwrap_or(&trimmed)
        .to_string()
}

fn strip_cursor_prefix(id: &str) -> &str {
    id.strip_prefix("cursor-").unwrap_or(id)
}

fn strip_effort_suffix(id: &str) -> Option<String> {
    for suffix in EFFORT_SUFFIXES {
        if let Some(stem) = id.strip_suffix(suffix) {
            if !stem.is_empty() {
                return Some(stem.to_string());
            }
        }
    }
    None
}

fn strip_md_link(name: &str) -> String {
    let name = name.trim();
    if let Some(rest) = name.strip_prefix('[') {
        if let Some((label, _)) = rest.split_once("](") {
            return label.trim().to_string();
        }
    }
    name.to_string()
}

fn parse_usd(cell: &str) -> Option<f64> {
    let cleaned = cell.trim().trim_start_matches('$').replace(',', "");
    if cleaned.is_empty() || cleaned == "-" {
        return None;
    }
    let n: f64 = cleaned.parse().ok()?;
    if n >= 0.0 && n.is_finite() {
        Some(n)
    } else {
        None
    }
}

fn ids_for_model_name(name: &str) -> Vec<String> {
    let lower = name.to_lowercase();
    let fast = lower.contains("(fast)") || lower.contains("fast mode") || lower.contains("fast)");
    let mut core = lower;
    if let Some(idx) = core.find('(') {
        core.truncate(idx);
    }
    core = core
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '-' { c } else { '-' })
        .collect::<String>();
    while core.contains("--") {
        core = core.replace("--", "-");
    }
    let core = core.trim_matches('-').to_string();
    if core.is_empty() {
        return Vec::new();
    }
    let mut ids = vec![core.clone()];
    if fast {
        ids.push(format!("{core}-fast"));
    }
    if core.contains('.') {
        ids.push(core.replace('.', "-"));
        if fast {
            ids.push(format!("{}-fast", core.replace('.', "-")));
        }
    }
    // "claude-4.6-sonnet" ↔ "claude-sonnet-4.6"
    if let Some(rest) = core.strip_prefix("claude-") {
        let parts: Vec<&str> = rest.split('-').collect();
        if parts.len() >= 2 {
            let family = parts[parts.len() - 1];
            let ver = parts[..parts.len() - 1].join("-");
            ids.push(format!("claude-{family}-{ver}"));
            ids.push(format!("claude-{family}-{}", ver.replace('.', "-")));
        }
    }
    ids.push(format!("cursor-{core}"));
    if fast {
        ids.push(format!("cursor-{core}-fast"));
    }
    ids
}

#[derive(Deserialize)]
struct PriceOverride {
    input_usd_per_mtok: f64,
    #[serde(default)]
    output_usd_per_mtok: Option<f64>,
    #[serde(default)]
    cache_read_usd_per_mtok: Option<f64>,
    #[serde(default)]
    cache_write_usd_per_mtok: Option<f64>,
}

fn merge_overrides(
    prices: &mut HashMap<String, (f64, PriceSource)>,
    extras: &mut HashMap<String, RateExtra>,
    path: &std::path::Path,
    source: PriceSource,
) {
    let Ok(bytes) = std::fs::read(path) else {
        return;
    };
    let Ok(value) = serde_json::from_slice::<Value>(&bytes) else {
        return;
    };
    let Some(obj) = value.as_object() else {
        return;
    };
    for (id, spec) in obj {
        if id.starts_with('_') {
            continue;
        }
        let (usd, extra) = if let Some(n) = spec.as_f64() {
            (n, RateExtra::default())
        } else if let Ok(row) = serde_json::from_value::<PriceOverride>(spec.clone()) {
            (
                row.input_usd_per_mtok,
                RateExtra {
                    output: row.output_usd_per_mtok,
                    cache_read: row.cache_read_usd_per_mtok,
                    cache_write: row.cache_write_usd_per_mtok,
                },
            )
        } else {
            continue;
        };
        if usd >= 0.0 && usd.is_finite() {
            let key = normalize(id);
            prices.insert(key.clone(), (usd, source));
            if extra.output.is_some() || extra.cache_read.is_some() || extra.cache_write.is_some() {
                extras.insert(key, extra);
            }
        }
    }
}

fn merge_models_dev(obj: &mut serde_json::Map<String, Value>, body: &str) {
    let Ok(root) = serde_json::from_str::<Value>(body) else {
        return;
    };
    let Some(providers) = root.as_object() else {
        return;
    };
    let catalog: Vec<String> = CATALOG.iter().map(|(id, _, _)| normalize(id)).collect();
    for provider in providers.values() {
        let Some(models) = provider.get("models").and_then(Value::as_object) else {
            continue;
        };
        for (id, spec) in models {
            let Some(cost) = spec.get("cost") else {
                continue;
            };
            let Some(input) = cost.get("input").and_then(Value::as_f64) else {
                continue;
            };
            if !(input >= 0.0 && input.is_finite()) {
                continue;
            }
            let key = normalize(id);
            if !catalog.iter().any(|c| c == &key) && !obj.contains_key(&key) {
                continue;
            }
            let mut row = serde_json::Map::new();
            row.insert("input_usd_per_mtok".into(), serde_json::json!(input));
            if let Some(n) = cost.get("output").and_then(Value::as_f64) {
                row.insert("output_usd_per_mtok".into(), serde_json::json!(n));
            }
            if let Some(n) = cost
                .get("cache_read")
                .or_else(|| cost.get("cacheRead"))
                .and_then(Value::as_f64)
            {
                row.insert("cache_read_usd_per_mtok".into(), serde_json::json!(n));
            }
            if let Some(n) = cost
                .get("cache_write")
                .or_else(|| cost.get("cacheWrite"))
                .and_then(Value::as_f64)
            {
                row.insert("cache_write_usd_per_mtok".into(), serde_json::json!(n));
            }
            obj.insert(key, Value::Object(row));
        }
    }
}

impl PriceBook {
    pub(crate) fn overlay_rates(&self, matched_id: &str, mut rates: crate::TierRates) -> crate::TierRates {
        if let Some(ex) = self.extras.get(matched_id) {
            if let Some(output) = ex.output {
                rates.output = output;
                rates.thinking = output;
            }
            if let Some(cache_read) = ex.cache_read {
                rates.cache_read = cache_read;
            }
            if let Some(cache_write) = ex.cache_write {
                rates.cache_write_5m = cache_write;
                rates.cache_write_1h = cache_write;
            }
        }
        rates
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ctx_store::CtxPaths;

    fn book(default: &str) -> (tempfile::TempDir, PriceBook) {
        let dir = tempfile::tempdir().unwrap();
        let paths = CtxPaths::from_root(dir.path().to_path_buf());
        (dir, PriceBook::load(&paths, default))
    }

    #[test]
    fn sonnet_alias_uses_catalog_input_price() {
        let (_dir, book) = book("");
        assert_eq!(book.input_usd_per_mtok("claude-sonnet-4-6"), Some(3.0));
        assert_eq!(book.avoided_usd("claude-sonnet-4-6", 1_000_000), Some(3.0));
        assert_eq!(book.avoided_usd("claude-sonnet-4-6", 500_000), Some(1.5));
    }

    #[test]
    fn unknown_and_auto_are_unpriced() {
        let (_dir, book) = book("");
        assert_eq!(book.input_usd_per_mtok("__unknown__"), None);
        assert_eq!(book.avoided_usd("", 1_000_000), None);
        assert_eq!(book.input_usd_per_mtok("default"), None);
        assert!(book.quote("auto").is_none());
    }

    #[test]
    fn default_billing_model_does_not_invent_unknown_dollars() {
        let (_dir, book) = book("deepseek-v4-flash");
        assert_eq!(book.input_usd_per_mtok("__unknown__"), None);
        assert_eq!(book.avoided_usd("__unknown__", 1_000_000), None);
        // Named models still resolve.
        assert_eq!(book.input_usd_per_mtok("deepseek-v4-flash"), Some(0.14));
    }

    #[test]
    fn cursor_grok_effort_maps_to_list_price() {
        let (_dir, book) = book("");
        assert_eq!(book.input_usd_per_mtok("cursor-grok-4.6-high"), Some(2.0));
        assert_eq!(book.input_usd_per_mtok("cursor-grok-4.6-fast"), Some(4.0));
    }

    #[test]
    fn prices_json_overrides_catalog() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("prices.json"),
            r#"{ "claude-sonnet-4": { "input_usd_per_mtok": 9.0 }, "custom-mini": 0.05 }"#,
        )
        .unwrap();
        let paths = CtxPaths::from_root(dir.path().to_path_buf());
        let book = PriceBook::load(&paths, "");
        assert_eq!(book.input_usd_per_mtok("claude-sonnet-4-6"), Some(9.0));
        assert_eq!(book.input_usd_per_mtok("custom-mini"), Some(0.05));
    }

    #[test]
    fn provider_prefix_is_stripped() {
        let (_dir, book) = book("");
        assert_eq!(
            book.input_usd_per_mtok("anthropic/claude-sonnet-4-6"),
            Some(3.0)
        );
    }

    #[test]
    fn auto_and_unknown_have_no_quote() {
        let (_dir, book) = book("");
        for id in ["default", "__unknown__", "auto", ""] {
            assert!(book.quote(id).is_none(), "{id}");
        }
    }

    #[test]
    fn display_name_uses_other_and_auto() {
        assert_eq!(PriceBook::display_name("__unknown__"), "Other");
        assert_eq!(PriceBook::display_name(""), "Other");
        assert_eq!(PriceBook::display_name("default"), "Auto");
        assert_eq!(PriceBook::display_name("auto"), "Auto");
        assert_eq!(PriceBook::display_name("grok-4.6"), "Grok 4.6");
        assert_eq!(PriceBook::display_name("cursor-grok-4.5-high"), "Grok 4.5");
    }

    #[test]
    fn canonical_id_merges_cursor_effort_variants() {
        let (_dir, book) = book("");
        assert_eq!(book.canonical_id("grok-4.5"), "grok-4.5");
        assert_eq!(book.canonical_id("cursor-grok-4.5"), "grok-4.5");
        assert_eq!(book.canonical_id("cursor-grok-4.5-high-fast"), "grok-4.5");
        assert_eq!(book.canonical_id("cursor-grok-4.6-high"), "grok-4.6");
        assert_eq!(book.canonical_id("claude-opus-5-thinking-max"), "claude-opus-5");
        assert_eq!(book.canonical_id("default"), "default");
        assert_eq!(book.canonical_id("__unknown__"), "__unknown__");
    }

    #[test]
    fn canonical_id_merges_even_when_cursor_key_is_priced() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("prices.official.json"),
            r#"{ "_meta": { "fetched_at": 1 }, "cursor-grok-4.5": 2.0, "grok-4.5": 2.0 }"#,
        )
        .unwrap();
        let paths = CtxPaths::from_root(dir.path().to_path_buf());
        let book = PriceBook::load(&paths, "");
        assert_eq!(book.canonical_id("cursor-grok-4.5"), "grok-4.5");
        assert_eq!(book.canonical_id("cursor-grok-4.5-high-fast"), "grok-4.5");
    }

    #[test]
    fn named_model_quotes_are_not_estimates() {
        let (_dir, book) = book("");
        let quote = book.quote("cursor-grok-4.6-high").expect("grok");
        assert_eq!(quote.source, PriceSource::Catalog);
        assert!(!quote.source.is_estimate());
    }

    #[test]
    fn official_cache_wins_over_catalog_and_reports_source() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("prices.official.json"),
            r#"{ "_meta": { "fetched_at": 1 }, "grok-4.6": 2.5 }"#,
        )
        .unwrap();
        let paths = CtxPaths::from_root(dir.path().to_path_buf());
        let book = PriceBook::load(&paths, "");
        let quote = book.quote("grok-4.6").expect("grok");
        assert_eq!(quote.usd_per_mtok, 2.5);
        assert_eq!(quote.source, PriceSource::Official);
    }

    #[test]
    fn cursor_markdown_table_fills_input_prices() {
        let md = "\
| Model | Provider | Input | Cache write | Cache read | Output |\n\
| --- | --- | --- | --- | --- | --- |\n\
| Grok 4.6 | Cursor | $2 | - | $0.5 | $6 |\n\
| Grok 4.6 (Fast) | Cursor | $4 | - | $1 | $12 |\n\
| [Claude 4.6 Sonnet](https://example.com) | Anthropic | $3 | $3.75 | $0.3 | $15 |\n";
        let parsed = parse_cursor_pricing_markdown(md);
        assert_eq!(parsed.get("grok-4.6"), Some(&2.0));
        assert_eq!(parsed.get("grok-4.6-fast"), Some(&4.0));
        assert_eq!(parsed.get("claude-4.6-sonnet"), Some(&3.0));
        assert_eq!(parsed.get("claude-sonnet-4.6"), Some(&3.0));
    }
}
