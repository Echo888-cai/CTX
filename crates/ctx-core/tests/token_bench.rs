//! Writes `target/token-bench.json` — full ingest (optimizer + envelope).

use std::path::PathBuf;

use ctx_core::{estimate_tokens, session_banner, Runtime};
use ctx_optimizer::{cow_working_set, outline_source, reduce_json_like};
use ctx_protocol::{CtxEvent, Harness, ToolRef};
use ctx_store::{CtxPaths, Store};
use serde::Serialize;

#[derive(Serialize)]
struct Row {
    name: String,
    kind: String,
    raw: u32,
    delivered: u32,
    avoided: u32,
    pct: u32,
    optimizer: String,
    kept_signal: Vec<String>,
}

fn pct(raw: u32, avoided: u32) -> u32 {
    if raw == 0 {
        0
    } else {
        ((avoided as f64 / raw as f64) * 100.0).round() as u32
    }
}

fn fixture(name: &str) -> String {
    let path = format!(
        "{}/../../benchmarks/fixtures/{name}",
        env!("CARGO_MANIFEST_DIR")
    );
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path}: {e}"))
}

fn ingest_shell(rt: &Runtime, payload: String) -> ctx_core::IngestResult {
    let event = CtxEvent::tool_output("bench", Harness::Unknown, ToolRef::new("Bash"), payload);
    rt.ingest(event).unwrap()
}

fn signal_hits(text: &str, needles: &[&str]) -> Vec<String> {
    needles
        .iter()
        .filter(|n| text.contains(**n))
        .map(|s| (*s).to_string())
        .collect()
}

fn row(
    name: &str,
    kind: &str,
    raw: u32,
    delivered: u32,
    optimizer: &str,
    kept_signal: Vec<String>,
) -> Row {
    let avoided = raw.saturating_sub(delivered);
    Row {
        name: name.into(),
        kind: kind.into(),
        raw,
        delivered,
        avoided,
        pct: pct(raw, avoided),
        optimizer: optimizer.into(),
        kept_signal,
    }
}

#[test]
fn write_token_bench() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(CtxPaths::from_root(dir.path().to_path_buf())).unwrap();
    let rt = Runtime::open(store);
    let mut rows = Vec::new();

    let cases: &[(&str, &str, &[&str])] = &[
        (
            "cargo test fail",
            "cargo-test-fail.txt",
            &["401", "auth::login", "redirect_uri mismatch", "src/auth.rs"],
        ),
        (
            "cargo workspace pass",
            "cargo-workspace-pass.txt",
            &["17 passed", "ctx_core", "ctx_optimizer"],
        ),
        (
            "cargo compile error",
            "cargo-compile-error.txt",
            &["E0308", "expected `u32`", "could not compile"],
        ),
        (
            "pytest fail",
            "pytest-fail.txt",
            &["401 == 200", "redirect_uri mismatch", "test_auth.py:82"],
        ),
        (
            "jest fail",
            "jest-fail.txt",
            &["Expected: 200", "Received: 401"],
        ),
        (
            "nextest fail",
            "nextest-fail.txt",
            &["401", "must_keep_error"],
        ),
    ];

    for (label, file, needles) in cases {
        let payload = fixture(file);
        let r = ingest_shell(&rt, payload);
        let hits = signal_hits(&r.delivered, needles);
        assert_eq!(
            hits.len(),
            needles.len(),
            "{label} dropped diagnostics: missing {:?}\n{}",
            needles
                .iter()
                .filter(|n| !r.delivered.contains(**n))
                .collect::<Vec<_>>(),
            r.delivered
        );
        rows.push(row(
            label,
            "fixture",
            r.raw_tokens,
            r.delivered_tokens,
            r.optimizer.as_deref().unwrap_or("passthrough"),
            hits,
        ));
    }

    let live_workspace =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/live-workspace.txt");
    if let Ok(payload) = std::fs::read_to_string(&live_workspace) {
        if estimate_tokens(&payload) >= 80 {
            let r = ingest_shell(&rt, payload);
            rows.push(row(
                "live cargo test --workspace",
                "live",
                r.raw_tokens,
                r.delivered_tokens,
                r.optimizer.as_deref().unwrap_or("passthrough"),
                signal_hits(&r.delivered, &["passed", "failed"]),
            ));
        }
    }

    let live_opt = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/live-raw.txt");
    if let Ok(payload) = std::fs::read_to_string(&live_opt) {
        if estimate_tokens(&payload) >= 80 {
            let r = ingest_shell(&rt, payload);
            rows.push(row(
                "live cargo test ctx-optimizer",
                "live",
                r.raw_tokens,
                r.delivered_tokens,
                r.optimizer.as_deref().unwrap_or("passthrough"),
                signal_hits(&r.delivered, &["passed", "failed"]),
            ));
        }
    }

    let dup = ingest_shell(&rt, fixture("cargo-test-fail.txt"));
    rows.push(row(
        "duplicate cargo test fail",
        "duplicate",
        dup.raw_tokens,
        dup.delivered_tokens,
        dup.optimizer.as_deref().unwrap_or("passthrough"),
        vec![dup.uri.unwrap_or_default()],
    ));

    let runtime_src =
        std::fs::read_to_string(format!("{}/src/runtime.rs", env!("CARGO_MANIFEST_DIR"))).unwrap();
    let outline = outline_source("crates/ctx-core/src/runtime.rs", &runtime_src);
    rows.push(row(
        "file outline runtime.rs",
        "file",
        estimate_tokens(&runtime_src),
        estimate_tokens(&outline),
        "file-read",
        signal_hits(&outline, &["ingest", "fetch", "search"]),
    ));

    let items: Vec<_> = (0..200)
        .map(|i| serde_json::json!({"id": i, "body": "x".repeat(80), "ok": true}))
        .collect();
    let json = serde_json::to_string(&items).unwrap();
    let json_out = reduce_json_like(&json);
    rows.push(row(
        "MCP JSON 200 items",
        "mcp",
        estimate_tokens(&json),
        estimate_tokens(&json_out),
        "mcp",
        signal_hits(&json_out, &["200"]),
    ));

    let mut prev = String::from("running 80 tests\n");
    let mut curr = String::from("running 80 tests\n");
    for i in 0..80 {
        prev.push_str(&format!("test t{i} ... ok\n"));
        if i == 7 {
            curr.push_str("test t7 ... FAILED\n");
        } else {
            curr.push_str(&format!("test t{i} ... ok\n"));
        }
    }
    prev.push_str("test result: ok. 80 passed; 0 failed\n");
    curr.push_str("---- t7 stdout ----\nleft: 401\nright: 200\n");
    curr.push_str("test result: FAILED. 79 passed; 1 failed\n");
    let cow = cow_working_set(&prev, "ctx://shell/prev", &curr).expect("cow");
    rows.push(row(
        "CoW delta (re-run)",
        "cow",
        estimate_tokens(&curr),
        estimate_tokens(&cow),
        "cow",
        signal_hits(&cow, &["401", "FAILED"]),
    ));

    const OLD_BANNER: &str =
        "CTX is active. Full context stays local. The model receives a working set.\n\
         Need more? ctx_fetch(uri#frame) pages a named region. ctx_search finds pages.";
    rows.push(row(
        "session banner (per chat)",
        "always-on",
        estimate_tokens(OLD_BANNER),
        estimate_tokens(session_banner()),
        "banner",
        vec!["ctx_fetch".into()],
    ));

    const OLD_TOOLS: &str = "Page in a preserved CTX payload by virtual address. uri may include a frame: ctx://shell/abc#auth::login. Default is a working-set preview. query selects a frame or region. query=\"*\" returns the full page.\n\
         Read a file through CTX. Large files return an outline. Pass query to page in a region. query=\"*\" returns the full file.\n\
         Page-fault search: walk the frame table (test names, errors), then FTS. Returns ctx:// URIs including #frame addresses for ctx_fetch.\n\
         Show the current virtual context: HOT / WARM / COLD plus recent page URIs.\n\
         Explain today's avoided tokens by reason. Trust UI, not a black box.";
    const NEW_TOOLS: &str =
        "Page in a ctx:// URI. uri#frame or query selects a region. query=* full page.\n\
         Read a file. Large files return a symbol index. query=region, *=full.\n\
         Find stored pages/frames by test name or error. Returns ctx://#frame.\n\
         HOT / WARM / COLD working set and mapped page URIs. task ranks by overlap.\n\
         Today's avoided tokens by reason.";
    rows.push(row(
        "MCP tool descriptions (per chat)",
        "always-on",
        estimate_tokens(OLD_TOOLS),
        estimate_tokens(NEW_TOOLS),
        "mcp-schema",
        vec!["ctx_fetch".into()],
    ));

    let fixture_raw: u32 = rows
        .iter()
        .filter(|r| r.kind == "fixture")
        .map(|r| r.raw)
        .sum();
    let fixture_del: u32 = rows
        .iter()
        .filter(|r| r.kind == "fixture")
        .map(|r| r.delivered)
        .sum();

    #[derive(Serialize)]
    struct Report {
        totals: Row,
        rows: Vec<Row>,
    }
    let report = Report {
        totals: row(
            "all fixtures",
            "total",
            fixture_raw,
            fixture_del,
            "mix",
            vec![],
        ),
        rows,
    };

    let out = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/token-bench.json");
    std::fs::create_dir_all(out.parent().unwrap()).ok();
    std::fs::write(&out, serde_json::to_string_pretty(&report).unwrap()).unwrap();
    assert!(
        fixture_del * 2 < fixture_raw,
        "fixtures should at least halve tokens: raw={fixture_raw} delivered={fixture_del}"
    );
}
