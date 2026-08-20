use ctx_core::{parse_task, render_spine, Runtime, WorkingSet};

pub fn run(session: Option<&str>, task: Option<&str>, json: bool) -> anyhow::Result<()> {
    let rt = Runtime::open_default()?;
    let extra = task.map(parse_task).unwrap_or_default();
    let ws = WorkingSet::query(&rt.store, session, &extra)?;
    let epoch = render_spine(&rt.store, session);
    if json {
        let pages: Vec<serde_json::Value> = ws
            .recent_pages
            .iter()
            .map(|p| {
                serde_json::json!({
                    "uri": p.uri,
                    "layer": p.layer,
                    "label": p.label,
                    "tokens": p.tokens,
                    "harness": p.harness,
                    "frame": p.frame,
                })
            })
            .collect();
        println!(
            "{}",
            serde_json::json!({
                "task": ws.task,
                "pages": pages,
                "epoch": epoch,
            })
        );
        return Ok(());
    }
    println!("{}", ws.render());
    if !epoch.is_empty() {
        println!("{epoch}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use ctx_core::{RecentPage, WorkingSet};

    #[test]
    fn render_lists_uris() {
        let ws = WorkingSet {
            recent_pages: vec![RecentPage {
                uri: "ctx://shell/abc123def".into(),
                layer: "HOT",
                label: "Recent errors / tool output".into(),
                tokens: 90,
                ..Default::default()
            }],
            ..Default::default()
        };
        let out = ws.render();
        assert!(out.contains("ctx://shell/abc123def"), "{out}");
        assert!(out.contains("HOT"), "{out}");
        assert!(out.contains("ctx_search"), "{out}");
    }
}
