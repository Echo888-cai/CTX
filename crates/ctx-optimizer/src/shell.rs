use crate::ansi::{is_progress_line, strip_ansi};
use crate::compact::compact_block;
use crate::header::prepend_command_exit;
use crate::pipeline::{OptimizeInput, OptimizeOutput, Optimizer};

pub struct ShellGuard;

impl Optimizer for ShellGuard {
    fn apply(&self, input: &OptimizeInput<'_>) -> Option<OptimizeOutput> {
        if input.kind != "shell" {
            return None;
        }
        if input.raw_tokens < 80 {
            return None;
        }
        let command = input.metadata.get("command").and_then(|v| v.as_str());
        let text = with_exec_header(input, reduce_shell_with(input.payload, command));
        let out = OptimizeOutput::reduced_terminal("shell", text);
        if out.delivered_tokens + 40 >= input.raw_tokens {
            return None;
        }
        Some(out)
    }
}

pub fn reduce_shell(payload: &str) -> String {
    reduce_shell_with(payload, None)
}

pub fn reduce_shell_with(payload: &str, command: Option<&str>) -> String {
    let stripped = strip_ansi(payload);
    let cleaned = collapse_noise(&stripped);
    let kind = detect_kind(&cleaned, command);
    match kind {
        ShellKind::GitDiff => Reduced {
            body: crate::git::reduce_diff(&cleaned),
        },
        ShellKind::GitLog => Reduced {
            body: crate::git::reduce_log(&cleaned),
        },
        ShellKind::GitStatus => Reduced {
            body: crate::git::reduce_status(&cleaned),
        },
        ShellKind::Install => Reduced {
            body: crate::install::reduce(&cleaned, crate::install::InstallKind::Packages),
        },
        ShellKind::Tree => Reduced {
            body: crate::install::reduce(&cleaned, crate::install::InstallKind::Tree),
        },
        ShellKind::Docker => Reduced {
            body: crate::install::reduce(&cleaned, crate::install::InstallKind::Docker),
        },
        ShellKind::Bundler => Reduced {
            body: crate::install::reduce(&cleaned, crate::install::InstallKind::Bundler),
        },
        ShellKind::CargoTest => reduce_cargo(&cleaned),
        ShellKind::CargoBuild => reduce_cargo_build(&cleaned),
        ShellKind::Nextest => reduce_nextest(&cleaned),
        ShellKind::Pytest => reduce_pytest(&cleaned),
        ShellKind::Jest => reduce_jest(&cleaned),
        ShellKind::Vitest => reduce_vitest(&cleaned),
        ShellKind::GoTest => reduce_go(&cleaned),
        ShellKind::Tsc => reduce_tsc(&cleaned),
        ShellKind::Eslint => reduce_eslint(&cleaned),
        ShellKind::Grep => Reduced {
            body: crate::grep::reduce(&cleaned, crate::grep::SearchKind::Grep),
        },
        ShellKind::Listing => Reduced {
            body: crate::grep::reduce(&cleaned, crate::grep::SearchKind::Listing),
        },
        ShellKind::Generic => reduce_generic(&cleaned),
    }
    .body
}

fn with_exec_header(input: &OptimizeInput<'_>, body: String) -> String {
    let command = input
        .metadata
        .get("command")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let exit = input
        .metadata
        .get("exit_code")
        .and_then(ctx_protocol::json_i64);
    prepend_command_exit(command, exit, &body)
}

#[derive(Debug, Clone, Copy)]
enum ShellKind {
    GitDiff,
    GitLog,
    GitStatus,
    Install,
    Tree,
    Docker,
    Bundler,
    CargoTest,
    CargoBuild,
    Nextest,
    Pytest,
    Jest,
    Vitest,
    GoTest,
    Tsc,
    Eslint,
    Grep,
    Listing,
    Generic,
}

fn detect_kind(text: &str, command: Option<&str>) -> ShellKind {
    match crate::git::detect_git(text, command) {
        Some(crate::git::GitKind::Diff) => return ShellKind::GitDiff,
        Some(crate::git::GitKind::Log) => return ShellKind::GitLog,
        Some(crate::git::GitKind::Status) => return ShellKind::GitStatus,
        None => {}
    }
    if let Some(kind) = crate::install::detect(text, command) {
        return match kind {
            crate::install::InstallKind::Packages => ShellKind::Install,
            crate::install::InstallKind::Tree => ShellKind::Tree,
            crate::install::InstallKind::Docker => ShellKind::Docker,
            crate::install::InstallKind::Bundler => ShellKind::Bundler,
        };
    }
    if looks_like_vitest(text, command) {
        return ShellKind::Vitest;
    }
    if text.contains("error[E")
        || text.contains("could not compile")
        || text.contains("error: aborting due to")
    {
        return ShellKind::CargoBuild;
    }
    if text.contains("tests across")
        || text.contains("PASS [")
        || text.contains("FAIL [")
        || text.contains("Summary [")
    {
        return ShellKind::Nextest;
    }
    if text.contains("test result:") || (text.contains("running ") && text.contains(" tests")) {
        return ShellKind::CargoTest;
    }
    if text.contains("short test summary info")
        || (text.contains(" passed") && text.contains(" failed") && text.contains("===="))
        || (text.contains("FAILURES") && text.contains("test session starts"))
    {
        return ShellKind::Pytest;
    }
    if (text.contains("Tests:") || text.contains("Test Suites:"))
        && (text.contains("PASS") || text.contains("FAIL ") || text.contains("FAIL\n"))
    {
        return ShellKind::Jest;
    }
    if text.contains("--- FAIL:") || text.contains("--- PASS:") || text.contains("FAIL\t") {
        return ShellKind::GoTest;
    }
    if text.contains("error TS") {
        return ShellKind::Tsc;
    }
    if looks_like_eslint(text) {
        return ShellKind::Eslint;
    }
    if looks_like_cargo_status(text) {
        return ShellKind::CargoBuild;
    }
    if text
        .lines()
        .filter(|l| l.trim_start().starts_with("warning:"))
        .count()
        >= 3
    {
        return ShellKind::CargoBuild;
    }
    if let Some(kind) = crate::grep::detect(text, command) {
        return match kind {
            crate::grep::SearchKind::Grep => ShellKind::Grep,
            crate::grep::SearchKind::Listing => ShellKind::Listing,
        };
    }
    ShellKind::Generic
}

fn looks_like_vitest(text: &str, command: Option<&str>) -> bool {
    if command.is_some_and(|c| c.to_ascii_lowercase().contains("vitest")) {
        return true;
    }
    text.contains("Test Files")
        && (text.contains(" FAIL  ") || text.contains(" ✓ ") || text.contains(" × "))
}

fn looks_like_eslint(text: &str) -> bool {
    let mut errors = 0u32;
    for line in text.lines() {
        let t = line.trim();
        if t.contains("  error  ") || t.contains("  error\t") || t.starts_with("✖ ") {
            errors += 1;
        }
    }
    errors >= 2 || (text.contains("eslint") && errors >= 1)
}

fn looks_like_cargo_status(text: &str) -> bool {
    let mut status = 0u32;
    for line in text.lines() {
        let t = line.trim_start();
        if t.starts_with("Compiling ")
            || t.starts_with("Checking ")
            || t.starts_with("Finished `")
            || t.starts_with("Downloading ")
        {
            status += 1;
        }
    }
    status >= 3
}

struct Reduced {
    body: String,
}

fn collapse_noise(text: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut prev: Option<&str> = None;
    let mut repeat = 0u32;
    for line in text.lines() {
        if is_progress_line(line) {
            continue;
        }
        if line.trim().is_empty() {
            if matches!(out.last(), Some(s) if s.is_empty()) {
                continue;
            }
            out.push(String::new());
            prev = Some("");
            repeat = 0;
            continue;
        }
        if prev == Some(line) {
            repeat += 1;
            continue;
        }
        if repeat > 0 {
            if let Some(last) = out.last_mut() {
                last.push_str(&format!("  ×{}", repeat + 1));
            }
            repeat = 0;
        }
        out.push(line.to_string());
        prev = Some(line);
    }
    out.join("\n")
}

fn parse_running_target(line: &str) -> Option<String> {
    let t = line.trim();
    if let Some(rest) = t.strip_prefix("Doc-tests ") {
        return Some(format!("doc:{rest}"));
    }
    if !(t.contains("Running ")) {
        return None;
    }
    let file = t.rsplit('(').next()?.trim_end_matches(')');
    let name = file.rsplit('/').next().unwrap_or(file);
    if let Some((stem, hash)) = name.rsplit_once('-') {
        if hash.len() >= 8 && hash.chars().all(|c| c.is_ascii_hexdigit()) {
            return Some(stem.to_string());
        }
    }
    Some(name.to_string())
}

struct TestStats {
    passed: u32,
    failed: u32,
    ignored: u32,
}

fn parse_test_result(line: &str) -> Option<TestStats> {
    // test result: FAILED. 8 passed; 4 failed; 0 ignored; ...
    let rest = line.trim().strip_prefix("test result:")?;
    let rest = rest.trim();
    let passed = capture_count(rest, "passed")?;
    let failed = capture_count(rest, "failed")?;
    let ignored = capture_count(rest, "ignored").unwrap_or(0);
    Some(TestStats {
        passed,
        failed,
        ignored,
    })
}

fn capture_count(hay: &str, label: &str) -> Option<u32> {
    let needle = format!(" {label}");
    let idx = hay.find(&needle)?;
    let before = hay[..idx].trim_end();
    let n = before
        .rsplit(|c: char| !c.is_ascii_digit())
        .next()
        .filter(|s| !s.is_empty())?;
    n.parse().ok()
}

fn reduce_cargo(text: &str) -> Reduced {
    let mut current_target = String::from("unknown");
    let mut targets: Vec<(String, TestStats)> = Vec::new();
    let mut failures = Vec::new();
    let mut failed_names = Vec::new();
    let mut in_failure_section = false;
    let mut in_block = false;
    let mut block = String::new();
    let mut passed_total = 0u32;
    let mut failed_total = 0u32;
    let mut ignored_total = 0u32;

    for line in text.lines() {
        let t = line.trim();
        if let Some(name) = parse_running_target(line) {
            if in_block && !block.is_empty() {
                failures.push(std::mem::take(&mut block));
            }
            current_target = name;
            in_failure_section = false;
            in_block = false;
            continue;
        }
        if t.starts_with("test result:") {
            if in_block && !block.is_empty() {
                failures.push(std::mem::take(&mut block));
                in_block = false;
            }
            if let Some(stats) = parse_test_result(t) {
                passed_total += stats.passed;
                failed_total += stats.failed;
                ignored_total += stats.ignored;
                targets.push((current_target.clone(), stats));
            }
            in_failure_section = false;
            continue;
        }
        if t == "failures:" {
            if in_block && !block.is_empty() {
                failures.push(std::mem::take(&mut block));
                in_block = false;
            }
            in_failure_section = true;
            continue;
        }
        if t == "successes:" {
            in_failure_section = false;
            in_block = false;
            continue;
        }
        if let Some(rest) = t.strip_prefix("test ") {
            if rest.contains(" ... FAILED") {
                let name = rest.split(" ... ").next().unwrap_or(rest);
                failed_names.push(name.to_string());
            }
        }
        if in_failure_section
            && t.starts_with("---- ")
            && (t.ends_with(" stdout ----") || t.ends_with(" stderr ----"))
        {
            if in_block && !block.is_empty() {
                failures.push(std::mem::take(&mut block));
            }
            in_block = true;
            block = format!("{t}\n");
            continue;
        }
        if in_block {
            if t.starts_with("---- ") || t == "failures:" || t.starts_with("test result:") {
                failures.push(std::mem::take(&mut block));
                in_block = t.starts_with("---- ");
                if in_block {
                    block = format!("{t}\n");
                }
            } else if block.lines().count() < 40 {
                block.push_str(line);
                block.push('\n');
            }
        }
    }
    if in_block && !block.is_empty() {
        failures.push(block);
    }

    let mut body = format!("{passed_total} passed, {failed_total} failed, {ignored_total} ignored");
    let live: Vec<_> = targets
        .iter()
        .filter(|(_, s)| s.passed + s.failed + s.ignored > 0)
        .collect();
    if live.len() > 1 {
        body.push_str(&format!("  {} targets\n", live.len()));
        if failed_total == 0 {
            let names: Vec<&str> = live.iter().take(24).map(|(n, _)| n.as_str()).collect();
            body.push_str("crates: ");
            body.push_str(&names.join(" "));
            body.push('\n');
        } else {
            for (name, stats) in live.iter().filter(|(_, s)| s.failed > 0).take(16) {
                body.push_str(&format!(
                    "  {name}  {} failed, {} passed\n",
                    stats.failed, stats.passed
                ));
            }
        }
    } else {
        body.push('\n');
    }

    if failures.is_empty() && failed_names.is_empty() {
        // counts already say 0 failed
    } else if !failures.is_empty() {
        body.push('\n');
        for f in failures.iter().take(8) {
            body.push_str(&compact_block(f.trim(), 18));
            body.push_str("\n\n");
        }
        if failures.len() > 8 {
            body.push_str(&format!("… {} more failures\n", failures.len() - 8));
        }
    } else {
        body.push('\n');
        for name in failed_names.iter().take(20) {
            body.push_str(&format!("# {name}\n"));
        }
    }
    Reduced { body }
}

fn reduce_cargo_build(text: &str) -> Reduced {
    let mut compiling = 0u32;
    let mut checking = 0u32;
    let mut errors = Vec::new();
    let mut warnings: Vec<(String, u32)> = Vec::new();
    let mut block = String::new();
    let mut kind: Option<&str> = None;
    let mut finished = None;

    fn flush(
        kind: &mut Option<&str>,
        block: &mut String,
        errors: &mut Vec<String>,
        warnings: &mut Vec<(String, u32)>,
    ) {
        if block.is_empty() {
            return;
        }
        let text = std::mem::take(block);
        match *kind {
            Some("error") => errors.push(text),
            Some("warning") => {
                let title = text.lines().next().unwrap_or("warning").to_string();
                if let Some((_, n)) = warnings.iter_mut().find(|(t, _)| *t == title) {
                    *n += 1;
                } else {
                    warnings.push((title, 1));
                }
            }
            _ => {}
        }
        *kind = None;
    }

    for line in text.lines() {
        let t = line.trim_start();
        if t.starts_with("Compiling ") {
            compiling += 1;
            continue;
        }
        if t.starts_with("Checking ") {
            checking += 1;
            continue;
        }
        if t.starts_with("Finished ") {
            finished = Some(t.to_string());
            continue;
        }
        if t.starts_with("error[") || t.starts_with("error: ") {
            flush(&mut kind, &mut block, &mut errors, &mut warnings);
            kind = Some("error");
            block = format!("{line}\n");
            continue;
        }
        if t.starts_with("warning:") {
            flush(&mut kind, &mut block, &mut errors, &mut warnings);
            kind = Some("warning");
            block = format!("{line}\n");
            continue;
        }
        if kind.is_some() && block.lines().count() < 16 {
            block.push_str(line);
            block.push('\n');
        }
    }
    flush(&mut kind, &mut block, &mut errors, &mut warnings);

    let mut body = String::new();
    if compiling + checking > 0 {
        body.push_str(&format!("checked {checking}, compiled {compiling}\n"));
    }
    if let Some(f) = finished {
        body.push_str(&f);
        body.push('\n');
    }
    if errors.is_empty() {
        body.push_str("0 compile errors\n");
    } else {
        body.push_str(&format!("{} errors\n\n", errors.len()));
        for e in errors.iter().take(8) {
            body.push_str(&compact_block(e.trim(), 16));
            body.push_str("\n\n");
        }
    }
    if !warnings.is_empty() {
        body.push_str(&format!("{} warnings\n", warnings.len()));
        for (title, n) in warnings.iter().take(8) {
            if *n > 1 {
                body.push_str(&format!("  {title}  ×{n}\n"));
            } else {
                body.push_str(&format!("  {title}\n"));
            }
        }
    }
    Reduced { body }
}

fn reduce_nextest(text: &str) -> Reduced {
    let mut fails = Vec::new();
    let mut summary = None;
    let mut in_fail = false;
    let mut block = String::new();
    let mut passed = 0u32;
    let mut failed = 0u32;

    for line in text.lines() {
        let t = line.trim();
        if t.starts_with("Summary ") || t.contains("tests run:") {
            summary = Some(t.to_string());
        }
        if t.contains("PASS [") {
            passed += 1;
            if in_fail && !block.is_empty() {
                fails.push(std::mem::take(&mut block));
                in_fail = false;
            }
            continue;
        }
        if t.contains("FAIL [") {
            if in_fail && !block.is_empty() {
                fails.push(std::mem::take(&mut block));
            }
            in_fail = true;
            failed += 1;
            block = format!("{t}\n");
            continue;
        }
        if in_fail && block.lines().count() < 24 {
            block.push_str(line);
            block.push('\n');
        }
    }
    if in_fail && !block.is_empty() {
        fails.push(block);
    }

    let mut body = String::new();
    if let Some(s) = summary {
        body.push_str(&s);
        body.push('\n');
    } else {
        body.push_str(&format!("{passed} passed, {failed} failed\n"));
    }
    if !fails.is_empty() {
        body.push('\n');
        for f in fails.iter().take(8) {
            body.push_str(&compact_block(f.trim(), 16));
            body.push_str("\n\n");
        }
    }
    Reduced { body }
}

fn reduce_pytest(text: &str) -> Reduced {
    let mut failed_lines = Vec::new();
    let mut summary = None;
    let mut in_failures = false;
    let mut failure_blocks = Vec::new();
    let mut block = String::new();

    for line in text.lines() {
        let t = line.trim();
        if t.contains("short test summary info") {
            if !block.is_empty() {
                failure_blocks.push(std::mem::take(&mut block));
            }
            in_failures = false;
        }
        if t.starts_with("FAILED ") || t.starts_with("ERROR ") {
            failed_lines.push(t.to_string());
        }
        if t.starts_with('=')
            && (t.contains("failed") || t.contains("passed") || t.contains("error"))
            && !t.contains("FAILURES")
            && !t.contains("ERRORS")
            && !t.contains("test session")
        {
            summary = Some(t.trim_matches('=').trim().to_string());
        }
        if (t.contains("FAILURES") || t.contains("ERRORS")) && t.contains('=') {
            in_failures = true;
            continue;
        }
        if in_failures {
            if t.starts_with("____") && !block.is_empty() {
                failure_blocks.push(std::mem::take(&mut block));
            }
            if block.lines().count() < 28 {
                block.push_str(line);
                block.push('\n');
            }
        }
    }
    if !block.is_empty() {
        failure_blocks.push(block);
    }

    let mut body = String::new();
    if let Some(s) = summary {
        body.push_str(&s);
        body.push('\n');
    }
    if !failure_blocks.is_empty() {
        body.push('\n');
        for f in failure_blocks.iter().take(8) {
            body.push_str(&compact_block(f.trim(), 16));
            body.push_str("\n\n");
        }
    } else if !failed_lines.is_empty() {
        body.push('\n');
        for f in failed_lines.iter().take(20) {
            body.push_str(f);
            body.push('\n');
        }
    }
    Reduced { body }
}

fn reduce_jest(text: &str) -> Reduced {
    let mut summary = None;
    let mut suites = None;
    let mut fail_blocks = Vec::new();
    let mut block = String::new();
    let mut in_fail = false;

    for line in text.lines() {
        let t = line.trim();
        if t.starts_with("Tests:") {
            summary = Some(t.to_string());
        }
        if t.starts_with("Test Suites:") {
            suites = Some(t.to_string());
        }
        if t.starts_with("FAIL ") {
            if in_fail && !block.is_empty() {
                fail_blocks.push(std::mem::take(&mut block));
            }
            in_fail = true;
            block = format!("{t}\n");
            continue;
        }
        if in_fail {
            if t.starts_with("PASS ") || t.starts_with("Tests:") || t.starts_with("Test Suites:") {
                fail_blocks.push(std::mem::take(&mut block));
                in_fail = false;
            } else if block.lines().count() < 24 {
                block.push_str(line);
                block.push('\n');
            }
        }
    }
    if in_fail && !block.is_empty() {
        fail_blocks.push(block);
    }

    let mut body = String::new();
    if let Some(s) = suites {
        body.push_str(&s);
        body.push('\n');
    }
    if let Some(s) = summary {
        body.push_str(&s);
        body.push('\n');
    }
    if !fail_blocks.is_empty() {
        body.push('\n');
        for f in fail_blocks.iter().take(8) {
            body.push_str(&compact_block(f.trim(), 16));
            body.push_str("\n\n");
        }
    }
    Reduced { body }
}

fn reduce_go(text: &str) -> Reduced {
    let mut fails = Vec::new();
    let mut summaries = Vec::new();
    let mut block = String::new();
    let mut in_fail = false;
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with("--- FAIL:") {
            if in_fail && !block.is_empty() {
                fails.push(std::mem::take(&mut block));
            }
            in_fail = true;
            block = format!("{t}\n");
            continue;
        }
        if in_fail {
            if t.starts_with("--- PASS:")
                || t.starts_with("--- FAIL:")
                || t.starts_with("FAIL\t")
                || t.starts_with("ok\t")
                || t.starts_with("ok  ")
            {
                fails.push(std::mem::take(&mut block));
                in_fail = t.starts_with("--- FAIL:");
                if in_fail {
                    block = format!("{t}\n");
                }
            } else if block.lines().count() < 24 {
                block.push_str(line);
                block.push('\n');
            }
        }
        if t.starts_with("FAIL\t")
            || ((t.starts_with("ok  ") || t.starts_with("ok\t")) && t.contains('\t'))
        {
            summaries.push(t.to_string());
        }
    }
    if in_fail && !block.is_empty() {
        fails.push(block);
    }
    let mut body = String::new();
    for s in summaries.iter().filter(|s| s.starts_with("FAIL")).take(16) {
        body.push_str(s);
        body.push('\n');
    }
    if summaries.iter().any(|s| s.starts_with("ok")) {
        let ok = summaries.iter().filter(|s| s.starts_with("ok")).count();
        body.push_str(&format!("{ok} packages ok\n"));
    }
    if !fails.is_empty() {
        body.push('\n');
        for f in fails.iter().take(8) {
            body.push_str(&compact_block(f.trim(), 16));
            body.push_str("\n\n");
        }
    }
    Reduced { body }
}

fn reduce_tsc(text: &str) -> Reduced {
    let mut errors = Vec::new();
    let mut found = None;
    for line in text.lines() {
        let t = line.trim();
        if t.contains("error TS") {
            errors.push(t.to_string());
        }
        if t.starts_with("Found ") && t.contains("error") {
            found = Some(t.to_string());
        }
    }
    let mut body = String::new();
    if let Some(s) = found {
        body.push_str(&s);
        body.push('\n');
    } else {
        body.push_str(&format!("{} errors\n", errors.len()));
    }
    body.push('\n');
    for e in errors.iter().take(24) {
        body.push_str(e);
        body.push('\n');
    }
    if errors.len() > 24 {
        body.push_str(&format!("… {} more\n", errors.len() - 24));
    }
    Reduced { body }
}

fn reduce_eslint(text: &str) -> Reduced {
    let mut errors = Vec::new();
    let mut summary = None;
    let mut file = "";
    for line in text.lines() {
        let t = line.trim();
        if (t.starts_with('/') || t.ends_with(".ts") || t.ends_with(".js") || t.ends_with(".tsx"))
            && !t.contains("  error")
            && !t.contains("  warning")
        {
            file = t;
            continue;
        }
        if t.contains("  error  ") || t.contains("  error\t") {
            if file.is_empty() {
                errors.push(t.to_string());
            } else {
                errors.push(format!("{file}  {t}"));
            }
        }
        if t.starts_with('✖') || (t.contains("problem") && t.contains("error")) {
            summary = Some(t.to_string());
        }
    }
    let mut body = String::new();
    if let Some(s) = summary {
        body.push_str(&s);
        body.push('\n');
    } else {
        body.push_str(&format!("{} errors\n", errors.len()));
    }
    body.push('\n');
    for e in errors.iter().take(24) {
        body.push_str(e);
        body.push('\n');
    }
    Reduced { body }
}

fn reduce_generic(text: &str) -> Reduced {
    let raw = crate::tokens::estimate_tokens(text);
    Reduced {
        body: crate::compact::diagnostic_ranked(text, &[], crate::budget::cap(raw)),
    }
}

fn reduce_vitest(text: &str) -> Reduced {
    let mut summary = Vec::new();
    let mut fails = Vec::new();
    let mut block = String::new();
    let mut in_fail = false;
    for line in text.lines() {
        let t = line.trim_end();
        if t.contains("Test Files") || t.contains("Tests ") || t.starts_with("Duration") {
            summary.push(t.trim().to_string());
        }
        if t.trim_start().starts_with("FAIL ") || t.trim_start().starts_with("FAIL  ") {
            if in_fail && !block.is_empty() {
                fails.push(std::mem::take(&mut block));
            }
            in_fail = true;
            block = format!("{t}\n");
            continue;
        }
        if in_fail {
            if t.trim_start().starts_with("PASS ")
                || t.trim_start().starts_with("FAIL ")
                || t.contains("Test Files")
            {
                fails.push(std::mem::take(&mut block));
                in_fail = t.trim_start().starts_with("FAIL");
                if in_fail {
                    block = format!("{t}\n");
                }
            } else if block.lines().count() < 24 {
                block.push_str(line);
                block.push('\n');
            }
        }
    }
    if in_fail && !block.is_empty() {
        fails.push(block);
    }
    let mut body = String::new();
    for s in &summary {
        body.push_str(s);
        body.push('\n');
    }
    if !fails.is_empty() {
        body.push('\n');
        for f in fails.iter().take(8) {
            body.push_str(&crate::compact::compact_block(f.trim(), 16));
            body.push_str("\n\n");
        }
    }
    Reduced { body }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::{OptimizeInput, Optimizer};
    use crate::tokens::estimate_tokens;

    fn fixture(name: &str) -> String {
        let path = format!(
            "{}/../../benchmarks/fixtures/{name}",
            env!("CARGO_MANIFEST_DIR")
        );
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path}: {e}"))
    }

    fn reduced(name: &str) -> String {
        reduce_shell(&fixture(name))
    }

    #[test]
    fn cargo_workspace_pass_summarizes_crates() {
        let out = reduced("cargo-workspace-pass.txt");
        assert!(out.contains("17 passed"), "{out}");
        assert!(out.contains("ctx_core"), "{out}");
        assert!(out.contains("ctx_optimizer"), "{out}");
        assert!(!out.contains("wraps_npm_test ... ok"), "{out}");
        assert!(out.contains("0 failed"), "{out}");
        let raw_t = estimate_tokens(&fixture("cargo-workspace-pass.txt"));
        let out_t = estimate_tokens(&out);
        assert!(out_t < raw_t / 2, "raw={raw_t} out={out_t}\n{out}");
    }

    #[test]
    fn cargo_fail_keeps_assertion_drops_passing_stdout() {
        let out = reduced("cargo-test-fail.txt");
        assert!(out.contains("401"), "{out}");
        assert!(out.contains("auth::login"), "{out}");
        assert!(out.contains("redirect_uri mismatch"), "{out}");
        assert!(
            !out.contains("this passing test printed a lot of noise"),
            "must not treat passing stdout as a failure:\n{out}"
        );
        assert!(!out.contains("foo::bar ... ok"), "{out}");
    }

    #[test]
    fn compile_error_keeps_rustc_span() {
        let out = reduced("cargo-compile-error.txt");
        assert!(out.contains("E0308"), "{out}");
        assert!(out.contains("expected `u32`"), "{out}");
        assert!(out.contains("could not compile"), "{out}");
        assert!(!out.contains("Checking ctx-store"), "{out}");
    }

    #[test]
    fn cargo_build_keeps_caused_by() {
        let raw = "\
   Compiling foo v0.1.0
error: failed to run custom build command for `libsqlite3-sys v0.35.0`

Caused by:
  process didn't exit successfully: `/tmp/build-script-build` (exit status: 101)
  --- stderr
  sandbox: cc not found

error: could not compile `foo` (lib) due to 1 previous error
";
        let out = reduce_shell(raw);
        assert!(out.contains("libsqlite3-sys"), "{out}");
        assert!(
            out.contains("cc not found") || out.contains("Caused by"),
            "{out}"
        );
        assert!(!out.contains("Compiling foo"), "{out}");
    }

    #[test]
    fn pytest_keeps_assert_and_summary() {
        let out = reduced("pytest-fail.txt");
        assert!(out.contains("4 failed, 819 passed"), "{out}");
        assert!(out.contains("401 == 200"), "{out}");
        assert!(out.contains("redirect_uri mismatch"), "{out}");
        assert!(
            out.contains("test_auth.py:82") || out.contains("where 401"),
            "must keep pytest failure body, not only FAILED lines:\n{out}"
        );
        assert!(!out.contains("test_health.py"), "{out}");
    }

    #[test]
    fn jest_keeps_expected_received() {
        let out = reduced("jest-fail.txt");
        assert!(out.contains("Expected: 200"), "{out}");
        assert!(out.contains("Received: 401"), "{out}");
        assert!(out.contains("4 failed, 819 passed"), "{out}");
        assert!(!out.contains("PASS src/health"), "{out}");
    }

    #[test]
    fn nextest_keeps_fail_stdout() {
        let out = reduced("nextest-fail.txt");
        assert!(out.contains("401"), "{out}");
        assert!(
            out.contains("must_keep_error") || out.contains("FAIL"),
            "{out}"
        );
        assert!(!out.contains("skips_nested"), "{out}");
    }

    #[test]
    fn includes_exit_code_when_known() {
        let payload = fixture("cargo-workspace-pass.txt");
        let meta = serde_json::json!({
            "exit_code": 0,
            "command": "cargo test"
        });
        let input = OptimizeInput {
            kind: "shell",
            tool_name: Some("Bash"),
            payload: &payload,
            metadata: &meta,
            raw_tokens: estimate_tokens(&payload),
        };
        let out = ShellGuard.apply(&input).expect("should virtualize");
        assert!(out.text.contains("exit 0"), "{}", out.text);
        assert!(out.text.contains("$ cargo test"), "{}", out.text);
    }

    #[test]
    fn strips_ansi_progress() {
        let mut raw = String::from("\x1b[32mok\x1b[0m\n");
        for _ in 0..50 {
            raw.push_str("⠋ compiling...\n");
        }
        raw.push_str(&"warning: unused\n".repeat(20));
        raw.push_str("error: boom\n");
        let out = reduce_shell(&raw.repeat(8));
        assert!(out.contains("error: boom") || out.contains("boom"));
        assert!(!out.contains('\u{1b}'));
    }

    #[test]
    fn tsc_keeps_error_lines() {
        let raw = "\
src/app.ts(12,5): error TS2345: Argument of type 'string' is not assignable to parameter of type 'number'.
src/lib.ts(3,1): error TS2304: Cannot find name 'foo'.
Found 2 errors.
";
        let out = reduce_shell(&raw.repeat(4));
        assert!(out.contains("TS2345"), "{out}");
        assert!(out.contains("src/app.ts"), "{out}");
        assert!(out.contains("TS2304"), "{out}");
    }

    #[test]
    fn cargo_fail_drops_backtrace() {
        let raw = "\
running 2 tests
test foo::bar ... ok
test auth::login ... FAILED

failures:

---- auth::login stdout ----
thread 'auth::login' panicked at src/auth.rs:82:5:
assertion `left == right` failed
  left: 401
  right: 200
stack backtrace:
   0: rust_begin_unwind
   1: core::panicking::panic_fmt
   2: app::auth::login
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace

test result: FAILED. 1 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
";
        let out = reduce_shell(raw);
        assert!(out.contains("401"), "{out}");
        assert!(out.contains("src/auth.rs:82"), "{out}");
        assert!(!out.contains("rust_begin_unwind"), "{out}");
        assert!(!out.contains("RUST_BACKTRACE"), "{out}");
    }

    #[test]
    fn npm_install_drops_registry_fetch() {
        let mut raw = String::new();
        for i in 0..30 {
            raw.push_str(&format!(
                "npm http fetch GET 200 https://registry.npmjs.org/pkg{i} 12ms\n"
            ));
        }
        raw.push_str("added 342 packages, and audited 343 packages in 12s\n");
        let out = reduce_shell_with(&raw, Some("npm install"));
        assert!(out.contains("added 342"), "{out}");
        assert!(!out.contains("registry.npmjs.org/pkg12"), "{out}");
    }

    #[test]
    fn vitest_keeps_fail_name() {
        let mut raw = String::from(" RUN  v1.6.0\n");
        for i in 0..40 {
            raw.push_str(&format!(" ✓ src/ok{i}.test.ts (1)\n"));
        }
        raw.push_str(" FAIL  src/auth.test.ts > login > returns 200\n");
        raw.push_str("AssertionError: expected 401 to be 200\n");
        raw.push_str(" Test Files  1 failed | 40 passed\n");
        raw.push_str("      Tests  1 failed | 40 passed\n");
        let out = reduce_shell_with(&raw, Some("npx vitest run"));
        assert!(out.contains("401") || out.contains("auth.test"), "{out}");
        assert!(out.contains("failed"), "{out}");
        assert!(!out.contains("src/ok12.test.ts"), "{out}");
    }

    #[test]
    fn rg_groups_hits_by_file() {
        let mut raw = String::new();
        for i in 0..40 {
            raw.push_str(&format!("src/auth.rs:{i}: let status = {i}\n"));
        }
        let out = reduce_shell_with(&raw, Some("rg status src"));
        assert!(out.contains("src/auth.rs"), "{out}");
        assert!(out.contains("matches"), "{out}");
        assert!(!out.contains("let status = 20"), "{out}");
    }
}
