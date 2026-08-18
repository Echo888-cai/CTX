use ctx_core::{fmt_compact, Config, CtxPaths, Runtime, Snapshot};

pub fn run() -> anyhow::Result<()> {
    let rt = Runtime::open_default()?;
    let snap = Snapshot::capture(&rt.store)?;
    println!("{}", render(rt.config.enabled, &snap));
    Ok(())
}

pub fn set_enabled(enabled: bool) -> anyhow::Result<()> {
    let paths = CtxPaths::default_home()?;
    let mut cfg = Config::load(&paths);
    cfg.enabled = enabled;
    cfg.save(&paths)?;
    if enabled {
        println!("CTX resumed. Tool output is virtualized again.");
    } else {
        println!("CTX paused. Tool output passes through. ctx resume to continue.");
    }
    Ok(())
}

pub fn render(enabled: bool, snap: &Snapshot) -> String {
    let enabled = if enabled { "running" } else { "paused" };
    let mut lines = vec![
        format!("CTX               {enabled}"),
        String::new(),
        "Today".into(),
        String::new(),
        format!("Raw context       {:>8}", fmt_compact(snap.today.raw)),
        format!("Delivered         {:>8}", fmt_compact(snap.today.delivered)),
        format!("Avoided           {:>8}", fmt_compact(snap.today.avoided)),
        String::new(),
        format!("Reduction         ↓{}%", snap.today.reduction_pct()),
    ];
    if !snap.by_harness_today.is_empty() {
        lines.push(String::new());
        for (h, t) in &snap.by_harness_today {
            lines.push(format!("{h:<16}  ↓{}%", t.reduction_pct()));
        }
    }
    lines.push(String::new());
    lines.push(format!("Pages stored      {}", snap.pages));
    lines.push(format!("Store             {}", fmt_bytes(snap.store_bytes)));
    lines.push("No context was lost.".into());
    lines.join("\n")
}

fn fmt_bytes(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1} MB", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1} KB", n as f64 / 1_000.0)
    } else {
        format!("{n} B")
    }
}

pub fn home(version: &str, snap: Option<&Snapshot>, enabled: bool) -> String {
    match snap {
        None => format!(
            "\
CTX  {version}
Virtual memory for AI context.

  ctx init       create ~/.ctx
  ctx setup      Claude / Cursor hooks
  ctx demo       see a page fault
  ctx doctor     wiring check

Same result. Less context."
        ),
        Some(snap) => {
            let state = if enabled { "running" } else { "paused" };
            format!(
                "\
CTX  {version}

  {state} · {} pages · ↓{}% today

  ctx status     today's efficiency
  ctx why        why those tokens stayed local
  ctx inspect    HOT / WARM / COLD + URIs
  ctx search     page-fault retrieval
  ctx doctor     wiring check

No context was lost.",
                snap.pages,
                snap.today.reduction_pct()
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_snap() -> Snapshot {
        Snapshot {
            today: Default::default(),
            week: Default::default(),
            by_harness_today: vec![],
            reasons_today: vec![],
            pages: 0,
            store_bytes: 0,
        }
    }

    #[test]
    fn home_first_run_points_to_init() {
        let out = home("0.1.0", None, true);
        assert!(out.contains("ctx init"));
        assert!(out.contains("ctx demo"));
        assert!(out.contains("ctx doctor"));
        assert!(!out.contains("clap"));
    }

    #[test]
    fn home_with_store_is_one_screen() {
        let mut snap = empty_snap();
        snap.pages = 9;
        let out = home("0.1.0", Some(&snap), true);
        assert!(out.contains("9 pages"));
        assert!(out.contains("ctx inspect"));
        assert!(out.contains("ctx search"));
        assert!(out.contains("ctx doctor"));
        assert!(out.contains("No context was lost."));
    }
}
