use ctx_core::{estimate_tokens, Runtime};
use ctx_protocol::{CtxEvent, Harness, ToolRef};

const DEMO: &str = include_str!("../../../benchmarks/fixtures/cargo-test-fail.txt");

pub fn run() -> anyhow::Result<()> {
    let rt = Runtime::open_default()?;
    let event = CtxEvent::tool_output(
        "demo",
        Harness::Unknown,
        ToolRef::new("Bash"),
        DEMO.to_string(),
    );
    let result = rt.ingest(event)?;
    let raw = estimate_tokens(DEMO);
    println!("CTX demo  (cargo test fixture)");
    println!();
    println!("Raw                    {raw}");
    println!("Delivered              {}", result.delivered_tokens);
    println!(
        "Avoided                {}   (↓{}%)",
        result.avoided_tokens,
        pct(raw, result.avoided_tokens)
    );
    if let Some(uri) = &result.uri {
        println!();
        println!("{uri}");
        println!("ctx fetch {uri} -q 401");
        println!("ctx search 401");
    }
    println!();
    println!("{}", result.delivered.trim());
    Ok(())
}

fn pct(raw: u32, avoided: u32) -> u32 {
    if raw == 0 {
        0
    } else {
        ((avoided as f64 / raw as f64) * 100.0).round() as u32
    }
}
