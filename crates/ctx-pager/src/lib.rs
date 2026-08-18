//! Working-set pager.
//!
//! Modest Working-Set Clock: referenced + recent → HOT; old unreferenced → COLD.
//! Mapped pages: when a task is known, rank store-wide by token overlap, not recency.

mod task;

pub use task::{extract_task, format_task, merge_tokens, overlap, parse_task};

use ctx_store::{Observation, PageMeta, Store};

const HOT_WINDOW_SECS: i64 = 15 * 60;
const WARM_WINDOW_SECS: i64 = 24 * 60 * 60;

#[derive(Debug, Clone, Default)]
pub struct Layer {
    pub tokens: u64,
    pub items: u64,
}

#[derive(Debug, Clone, Default)]
pub struct WorkingSet {
    pub hot: Layer,
    pub warm: Layer,
    pub cold: Layer,
    pub hot_breakdown: Vec<(String, u64)>,
    /// Mapped pages. Task-ranked when a task is known; otherwise newest first.
    pub recent_pages: Vec<RecentPage>,
    pub task: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct RecentPage {
    pub uri: String,
    pub layer: &'static str,
    pub label: String,
    pub tokens: u64,
    pub harness: String,
    pub frame: String,
}

impl WorkingSet {
    pub fn from_store(store: &Store, session_id: Option<&str>) -> ctx_store::Result<Self> {
        Self::query(store, session_id, &[])
    }

    pub fn query(
        store: &Store,
        session_id: Option<&str>,
        extra_task: &[String],
    ) -> ctx_store::Result<Self> {
        let now = now_secs();
        let obs = if let Some(session) = session_id {
            let mut rows = store.observations_for_session(session)?;
            if rows.is_empty() {
                rows = store.observations_since(now - WARM_WINDOW_SECS * 7)?;
            }
            rows
        } else {
            store.observations_since(now - WARM_WINDOW_SECS * 30)?
        };
        let mut ws = Self::from_observations(&obs, now, &git_paths_for(&obs));
        let stored = session_id
            .map(|id| store.session_task(id).unwrap_or_default())
            .unwrap_or_default();
        ws.task = merge_tokens(&parse_task(&stored), extra_task);
        let metas = store.recent_pages(200).unwrap_or_default();
        if ws.task.is_empty() {
            enrich_pages(&mut ws.recent_pages, &metas);
        } else {
            ws.recent_pages = select_mapped(&metas, &obs, &ws.task, now, 12);
        }
        Ok(ws)
    }

    pub fn from_observations(obs: &[Observation], now: i64, git_paths: &[String]) -> Self {
        classify(obs, now, git_paths)
    }

    /// Compact mapped set for SessionStart. Empty when there is nothing to map.
    pub fn render_mapped(&self) -> String {
        if self.recent_pages.is_empty() {
            return String::new();
        }
        let mut out = String::new();
        if !self.task.is_empty() {
            out.push_str(&self.task.join(" "));
            out.push('\n');
        }
        for p in &self.recent_pages {
            let addr = mapped_addr(p);
            out.push_str("  ");
            if !p.layer.is_empty() {
                out.push_str(p.layer);
                out.push_str("  ");
            }
            out.push_str(&addr);
            if !p.harness.is_empty() {
                out.push_str("  ");
                out.push_str(&p.harness);
            }
            out.push('\n');
        }
        out
    }

    pub fn render(&self) -> String {
        let mut lines = vec![
            "Current Virtual Context".into(),
            String::new(),
            format!(
                "HOT                       {:>8}    referenced · recent",
                compact(self.hot.tokens)
            ),
            "──────────────────────────────".into(),
        ];
        if self.hot_breakdown.is_empty() {
            lines.push("(none — no recent referenced pages)".into());
        } else {
            for (label, tokens) in &self.hot_breakdown {
                lines.push(format!("{label:<24} {:>8}", compact(*tokens)));
            }
        }
        lines.push(String::new());
        lines.push(format!(
            "WARM                      {:>8}    cooling",
            compact(self.warm.tokens)
        ));
        lines.push("──────────────────────────────".into());
        lines.push(format!("items                    {:>8}", self.warm.items));
        lines.push(String::new());
        lines.push(format!(
            "COLD                      {:>8}    old · unreferenced",
            compact(self.cold.tokens)
        ));
        if !self.task.is_empty() {
            lines.push(String::new());
            lines.push(format!("task  {}", self.task.join(" ")));
        }
        if !self.recent_pages.is_empty() {
            lines.push(String::new());
            lines.push("Pages".into());
            lines.push(String::new());
            for p in &self.recent_pages {
                let extra = if p.harness.is_empty() {
                    p.label.clone()
                } else {
                    format!("{}  {}", p.harness, p.label)
                };
                lines.push(format!(
                    "  {:<4}  {:<36} {:>8}  {}",
                    p.layer,
                    mapped_addr(p),
                    compact(p.tokens),
                    extra
                ));
            }
        }
        lines.push(String::new());
        lines.push(
            "The model sees HOT. ctx_fetch(uri, query) pages a region. ctx_search finds pages."
                .into(),
        );
        lines.join("\n")
    }
}

fn classify(obs: &[Observation], now: i64, git_paths: &[String]) -> WorkingSet {
    let mut ws = WorkingSet::default();
    let mut hot_map: std::collections::BTreeMap<String, u64> = std::collections::BTreeMap::new();
    let mut pages: Vec<RecentPage> = Vec::new();
    for o in obs {
        let age = now.saturating_sub(if o.accessed_at > 0 {
            o.accessed_at
        } else {
            o.created_at
        });
        let tokens = o.raw_tokens as u64;
        let referenced = observation_referenced(o, git_paths);
        let layer = clock_layer(referenced, age);
        match layer {
            ClockLayer::Hot => {
                ws.hot.tokens += tokens;
                ws.hot.items += 1;
                let key = o.tool_type.clone().unwrap_or_else(|| o.event_type.clone());
                *hot_map.entry(label_for(&key)).or_default() += o.delivered_tokens as u64;
            }
            ClockLayer::Warm => {
                ws.warm.tokens += tokens;
                ws.warm.items += 1;
            }
            ClockLayer::Cold => {
                ws.cold.tokens += tokens;
                ws.cold.items += 1;
            }
        }
        if let Some(uri) = o.uri.as_ref().filter(|u| !u.is_empty()) {
            pages.push(RecentPage {
                uri: uri.clone(),
                layer: layer.as_str(),
                label: label_for(o.tool_type.as_deref().unwrap_or(&o.event_type)),
                tokens: o.delivered_tokens as u64,
                harness: String::new(),
                frame: String::new(),
            });
        }
    }
    ws.hot_breakdown = hot_map.into_iter().collect();
    ws.recent_pages = dedupe_pages_newest_first(pages, 12);
    ws
}

fn dedupe_pages_newest_first(pages: Vec<RecentPage>, limit: usize) -> Vec<RecentPage> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for page in pages.into_iter().rev() {
        if seen.insert(page.uri.clone()) {
            out.push(page);
        }
        if out.len() >= limit {
            break;
        }
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClockLayer {
    Hot,
    Warm,
    Cold,
}

impl ClockLayer {
    fn as_str(self) -> &'static str {
        match self {
            Self::Hot => "HOT",
            Self::Warm => "WARM",
            Self::Cold => "COLD",
        }
    }
}

/// HOT = referenced + recent; COLD = old unreferenced; else WARM.
fn clock_layer(referenced: bool, age_secs: i64) -> ClockLayer {
    if referenced && age_secs <= HOT_WINDOW_SECS {
        ClockLayer::Hot
    } else if referenced || age_secs <= WARM_WINDOW_SECS {
        ClockLayer::Warm
    } else {
        ClockLayer::Cold
    }
}

pub fn observation_referenced(o: &Observation, git_paths: &[String]) -> bool {
    if o.referenced {
        return true;
    }
    is_referenced(
        o.tool_type.as_deref(),
        o.tool_name.as_deref(),
        "",
        None,
        o.source_path.as_deref(),
        git_paths,
    )
}

/// Referenced bit: error/fail/panic signal, nonzero shell exit, or path in git diff.
pub fn is_referenced(
    tool_type: Option<&str>,
    tool_name: Option<&str>,
    hint: &str,
    exit_code: Option<i64>,
    path: Option<&str>,
    git_paths: &[String],
) -> bool {
    if tool_type == Some("shell") && matches!(exit_code, Some(code) if code != 0) {
        return true;
    }
    if looks_signal(hint) || tool_name.is_some_and(looks_signal) {
        return true;
    }
    if let Some(path) = path {
        if git_paths.iter().any(|g| path_matches(g, path)) {
            return true;
        }
    }
    false
}

/// True for compiler/test failure text, not for "0 failed" summaries.
pub fn looks_signal(s: &str) -> bool {
    if s.contains("FAILED") || s.contains("PANIC") {
        return true;
    }
    let l = s.to_ascii_lowercase();
    l.contains("panic")
        || l.contains("error:")
        || l.contains("error[")
        || l.contains("exception")
        || l.contains("fatal:")
        || l.contains(" ... failed")
        || l.contains("fail [")
}

fn path_matches(git: &str, obs: &str) -> bool {
    let g = git.replace('\\', "/");
    let o = obs.replace('\\', "/");
    if g.is_empty() || o.is_empty() {
        return false;
    }
    o == g || o.ends_with(&g) || g.ends_with(&o)
}

/// Cheap, optional. Skips when cwd is not a git work tree. No timeout.
fn git_paths_for(obs: &[Observation]) -> Vec<String> {
    if !obs.iter().any(|o| o.source_path.is_some() && !o.referenced) {
        return Vec::new();
    }
    git_changed_paths()
}

/// Cheap, optional. Skips when cwd is not a git work tree.
fn git_changed_paths() -> Vec<String> {
    if !std::path::Path::new(".git").exists() {
        return Vec::new();
    }
    let output = std::process::Command::new("git")
        .args(["diff", "--name-only", "--no-ext-diff", "HEAD"])
        .env("GIT_OPTIONAL_LOCKS", "1")
        .output();
    let Ok(output) = output else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|l| !l.is_empty())
        .take(64)
        .map(str::to_string)
        .collect()
}

fn mapped_addr(p: &RecentPage) -> String {
    if p.frame.is_empty() || p.uri.contains('#') {
        p.uri.clone()
    } else {
        format!("{}#{}", p.uri, p.frame)
    }
}

fn enrich_pages(pages: &mut [RecentPage], metas: &[PageMeta]) {
    for p in pages {
        if let Some(m) = metas.iter().find(|m| m.uri == p.uri) {
            if p.harness.is_empty() {
                p.harness = m.harness.clone();
            }
            if p.frame.is_empty() {
                p.frame = m.summary.clone().unwrap_or_default();
            }
        }
    }
}

fn select_mapped(
    pages: &[PageMeta],
    obs: &[Observation],
    query: &[String],
    now: i64,
    limit: usize,
) -> Vec<RecentPage> {
    let mut latest: std::collections::HashMap<String, &Observation> =
        std::collections::HashMap::new();
    for o in obs {
        if let Some(uri) = o.uri.as_ref().filter(|u| !u.is_empty()) {
            latest.insert(uri.clone(), o);
        }
    }
    let git = git_paths_for(obs);
    let mut scored: Vec<(u32, i64, RecentPage)> = Vec::new();
    for page in pages {
        let (layer, tokens) = if let Some(o) = latest.get(&page.uri) {
            let age = now.saturating_sub(if o.accessed_at > 0 {
                o.accessed_at
            } else {
                o.created_at
            });
            let referenced = observation_referenced(o, &git);
            (clock_layer(referenced, age), o.delivered_tokens as u64)
        } else {
            let age = now.saturating_sub(page.created_at);
            (clock_layer(false, age), page.raw_tokens as u64)
        };
        let page_tokens = parse_task(&page.task);
        let ov = overlap(&page_tokens, query);
        if ov == 0 && layer == ClockLayer::Cold {
            continue;
        }
        let mut score = ov * 12;
        match layer {
            ClockLayer::Hot => score += 8,
            ClockLayer::Warm => score += 3,
            ClockLayer::Cold => {}
        }
        if ov > 0 {
            score += 6;
        }
        let age = now.saturating_sub(page.created_at);
        score += ((7 * 86_400 - age.clamp(0, 7 * 86_400)) / 86_400) as u32;
        scored.push((
            score,
            page.created_at,
            RecentPage {
                uri: page.uri.clone(),
                layer: layer.as_str(),
                label: label_for(&page.kind),
                tokens,
                harness: page.harness.clone(),
                frame: page.summary.clone().unwrap_or_default(),
            },
        ));
    }
    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.cmp(&a.1)));
    scored.into_iter().map(|(_, _, p)| p).take(limit).collect()
}

fn label_for(kind: &str) -> String {
    match kind {
        "shell" => "Recent errors / tool output".into(),
        "file" => "Active source".into(),
        "mcp" => "MCP results".into(),
        "search" => "Search".into(),
        other => other.to_string(),
    }
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn compact(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.2}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.2}K", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

pub fn start_of_today() -> i64 {
    let now = now_secs();
    match secs_since_local_midnight() {
        Some(elapsed) => now.saturating_sub(elapsed),
        None => now - now.rem_euclid(86_400),
    }
}

pub fn start_of_week() -> i64 {
    start_of_today() - 6 * 86_400
}

#[cfg(unix)]
fn secs_since_local_midnight() -> Option<i64> {
    // SAFETY: `tm` is written by localtime_r before any field is read.
    unsafe {
        let t = libc::time(std::ptr::null_mut());
        if t < 0 {
            return None;
        }
        let mut tm = std::mem::zeroed();
        if libc::localtime_r(&t, &mut tm).is_null() {
            return None;
        }
        Some(i64::from(tm.tm_hour) * 3600 + i64::from(tm.tm_min) * 60 + i64::from(tm.tm_sec))
    }
}

#[cfg(not(unix))]
fn secs_since_local_midnight() -> Option<i64> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use ctx_store::Observation;

    fn obs(
        id: i64,
        tool_type: &str,
        created_at: i64,
        referenced: bool,
        source_path: Option<&str>,
        hint: &str,
    ) -> Observation {
        Observation {
            id,
            session_id: "s".into(),
            event_type: "tool.output".into(),
            tool_type: Some(tool_type.into()),
            tool_name: Some("Bash".into()),
            uri: None,
            content_hash: "h".into(),
            raw_tokens: 100,
            delivered_tokens: 20,
            avoided_tokens: 80,
            optimizer: None,
            reasons: serde_json::json!([{"label": hint, "tokens": 80}]),
            created_at,
            referenced,
            source_path: source_path.map(str::to_string),
            accessed_at: 0,
        }
    }

    #[test]
    fn hot_is_referenced_and_recent() {
        let now = 1_000_000;
        let mut row = obs(1, "shell", now - 60, true, None, "ok");
        row.uri = Some("ctx://shell/abc123".into());
        let ws = WorkingSet::from_observations(&[row], now, &[]);
        assert_eq!(ws.hot.items, 1);
        assert_eq!(ws.warm.items, 0);
        assert_eq!(ws.cold.items, 0);
        assert_eq!(ws.recent_pages.len(), 1);
        assert_eq!(ws.recent_pages[0].layer, "HOT");
        assert_eq!(ws.recent_pages[0].uri, "ctx://shell/abc123");
    }

    #[test]
    fn recent_unreferenced_is_warm_not_hot() {
        let now = 1_000_000;
        let rows = vec![obs(1, "shell", now - 60, false, None, "ok")];
        let ws = WorkingSet::from_observations(&rows, now, &[]);
        assert_eq!(ws.hot.items, 0);
        assert_eq!(ws.warm.items, 1);
        assert_eq!(ws.cold.items, 0);
    }

    #[test]
    fn old_unreferenced_is_cold() {
        let now = 1_000_000;
        let rows = vec![obs(
            1,
            "shell",
            now - WARM_WINDOW_SECS - 10,
            false,
            None,
            "ok",
        )];
        let ws = WorkingSet::from_observations(&rows, now, &[]);
        assert_eq!(ws.hot.items, 0);
        assert_eq!(ws.cold.items, 1);
    }

    #[test]
    fn old_referenced_stays_warm() {
        let now = 1_000_000;
        let rows = vec![obs(
            1,
            "shell",
            now - WARM_WINDOW_SECS - 10,
            true,
            None,
            "error:",
        )];
        let ws = WorkingSet::from_observations(&rows, now, &[]);
        assert_eq!(ws.warm.items, 1);
        assert_eq!(ws.cold.items, 0);
    }

    #[test]
    fn recent_access_promotes_old_page_to_hot() {
        let now = 1_000_000;
        let mut row = obs(
            1,
            "shell",
            now - WARM_WINDOW_SECS - 10,
            true,
            None,
            "error:",
        );
        row.accessed_at = now - 30;
        row.uri = Some("ctx://shell/abc".into());
        let ws = WorkingSet::from_observations(&[row], now, &[]);
        assert_eq!(ws.hot.items, 1, "fetch ticks the clock into HOT");
        assert_eq!(ws.recent_pages[0].layer, "HOT");
    }

    #[test]
    fn git_path_marks_file_referenced() {
        let now = 1_000_000;
        let rows = vec![obs(1, "file", now - 30, false, Some("src/lib.rs"), "ok")];
        let git = vec!["src/lib.rs".into()];
        let ws = WorkingSet::from_observations(&rows, now, &git);
        assert_eq!(ws.hot.items, 1, "path in git diff + recent → HOT");
    }

    #[test]
    fn looks_signal_skips_zero_failed() {
        assert!(!looks_signal("17 passed, 0 failed, 0 ignored"));
        assert!(!looks_signal("No compile errors."));
        assert!(looks_signal("error: boom"));
        assert!(looks_signal("error[E0308]: mismatched types"));
        assert!(looks_signal("test auth::login ... FAILED"));
        assert!(looks_signal("thread panicked at"));
    }

    #[test]
    fn nonzero_shell_exit_is_referenced() {
        assert!(is_referenced(
            Some("shell"),
            Some("Bash"),
            "ok",
            Some(1),
            None,
            &[]
        ));
        assert!(!is_referenced(
            Some("shell"),
            Some("Bash"),
            "ok",
            Some(0),
            None,
            &[]
        ));
    }

    #[test]
    fn reasons_json_is_not_a_log_signal() {
        let now = 1_000_000;
        let mut row = obs(1, "shell", now - 60, false, None, "error:");
        row.reasons = serde_json::json!([{"label": "test output noise", "tokens": 80}]);
        let ws = WorkingSet::from_observations(&[row], now, &[]);
        assert_eq!(ws.hot.items, 0, "reasons JSON must not promote HOT");
        assert_eq!(ws.warm.items, 1);
    }

    #[test]
    fn today_is_within_the_last_local_day() {
        let t = start_of_today();
        let now = now_secs();
        assert!(t <= now, "t={t} now={now}");
        assert!(now - t < 86_400 + 3_600, "span {}", now - t);
    }

    fn page(
        uri: &str,
        task: &str,
        harness: &str,
        created_at: i64,
        summary: &str,
    ) -> ctx_store::PageMeta {
        ctx_store::PageMeta {
            uri: uri.into(),
            hash: "h".into(),
            kind: "shell".into(),
            summary: if summary.is_empty() {
                None
            } else {
                Some(summary.into())
            },
            raw_tokens: 100,
            created_at,
            task: task.into(),
            harness: harness.into(),
        }
    }

    #[test]
    fn task_matching_cold_page_beats_recent_unmatched() {
        let now = 1_000_000;
        let cold_uri = "ctx://shell/oldauth";
        let hot_uri = "ctx://shell/newfmt";
        let metas = vec![
            page(hot_uri, "fmt clippy", "cursor", now - 60, ""),
            page(
                cold_uri,
                "auth login",
                "claude-code",
                now - WARM_WINDOW_SECS - 10,
                "auth::login",
            ),
        ];
        let mut cold = obs(1, "shell", now - WARM_WINDOW_SECS - 10, false, None, "ok");
        cold.uri = Some(cold_uri.into());
        let mut hot = obs(2, "shell", now - 60, true, None, "ok");
        hot.uri = Some(hot_uri.into());
        let query = extract_task(&["fix auth login"]);
        let mapped = select_mapped(&metas, &[cold, hot], &query, now, 8);
        assert!(!mapped.is_empty(), "{mapped:?}");
        assert_eq!(mapped[0].uri, cold_uri, "{mapped:?}");
        assert_eq!(mapped[0].layer, "COLD");
        assert_eq!(mapped[0].harness, "claude-code");
        assert!(mapped[0].frame.contains("auth"), "{}", mapped[0].frame);
    }

    #[test]
    fn mapped_render_is_a_tiny_page_table() {
        let ws = WorkingSet {
            task: vec!["auth".into(), "login".into()],
            recent_pages: vec![RecentPage {
                uri: "ctx://shell/abc123".into(),
                layer: "COLD",
                label: "Recent errors / tool output".into(),
                tokens: 90,
                harness: "claude-code".into(),
                frame: "auth::login".into(),
            }],
            ..Default::default()
        };
        let out = ws.render_mapped();
        assert!(out.contains("auth login"), "{out}");
        assert!(out.contains("ctx://shell/abc123#auth::login"), "{out}");
        assert!(out.contains("claude-code"), "{out}");
        assert!(out.contains("COLD"), "{out}");
    }
}
