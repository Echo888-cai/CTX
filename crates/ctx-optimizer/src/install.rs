//! Package-manager, tree, docker, and bundler noise.
//!
//! Install logs are almost all progress. The model needs the summary, the
//! error, and the package that failed — not 400 HTTP fetch lines.

use crate::compact::{compact_block, is_diagnostic_line};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallKind {
    Packages,
    Tree,
    Docker,
    Bundler,
}

pub fn detect(text: &str, command: Option<&str>) -> Option<InstallKind> {
    if looks_like_tests(text) {
        return None;
    }
    if let Some(cmd) = command {
        let c = cmd.to_ascii_lowercase();
        if looks_like_tree_command(&c) {
            return Some(InstallKind::Tree);
        }
        if looks_like_docker_command(&c) {
            return Some(InstallKind::Docker);
        }
        if looks_like_bundler_command(&c) {
            return Some(InstallKind::Bundler);
        }
        if looks_like_install_command(&c) {
            return Some(InstallKind::Packages);
        }
    }
    if looks_like_tree(text) {
        return Some(InstallKind::Tree);
    }
    if looks_like_docker(text) {
        return Some(InstallKind::Docker);
    }
    if looks_like_bundler(text) {
        return Some(InstallKind::Bundler);
    }
    if looks_like_install(text) {
        return Some(InstallKind::Packages);
    }
    None
}

pub fn reduce(text: &str, kind: InstallKind) -> String {
    match kind {
        InstallKind::Packages => reduce_install(text),
        InstallKind::Tree => reduce_tree(text),
        InstallKind::Docker => reduce_docker(text),
        InstallKind::Bundler => reduce_bundler(text),
    }
}

fn looks_like_tests(text: &str) -> bool {
    text.contains("test result:")
        || text.contains("short test summary info")
        || text.contains("error[E")
        || text.contains("Tests:")
        || text.contains("Test Files")
}

fn looks_like_install_command(c: &str) -> bool {
    let needles = [
        "npm install",
        "npm i ",
        "yarn install",
        "yarn add",
        "pnpm install",
        "pnpm i ",
        "pnpm add",
        "bun install",
        "bun add",
        "cargo fetch",
        "cargo update",
        "pip install",
        "pip3 install",
        "uv pip install",
        "poetry install",
        "bundle install",
    ];
    needles.iter().any(|n| c.contains(n))
        || (c.contains("npm i") && !c.contains("npm init") && !c.contains("npm info"))
}

fn looks_like_tree_command(c: &str) -> bool {
    c.contains("cargo tree") || c.contains("npm ls") || c.contains("pnpm list")
}

fn looks_like_docker_command(c: &str) -> bool {
    c.contains("docker build")
        || c.contains("docker compose")
        || c.contains("docker-compose")
        || c.contains("podman build")
}

fn looks_like_bundler_command(c: &str) -> bool {
    c.contains("vite build")
        || c.contains("webpack")
        || c.contains("next build")
        || c.contains("turbo run build")
}

fn looks_like_install(text: &str) -> bool {
    let mut fetch = 0u32;
    let mut added = false;
    for line in text.lines() {
        let l = line.to_ascii_lowercase();
        if l.contains("http fetch")
            || l.contains("download ")
            || l.contains("downloading ")
            || l.contains("fetching ")
        {
            fetch += 1;
        }
        if l.contains("added ") && l.contains("package") {
            added = true;
        }
        if l.contains("audited ") && l.contains("package") {
            added = true;
        }
    }
    fetch >= 8 || (added && fetch >= 2) || (added && text.lines().count() > 40)
}

fn looks_like_tree(text: &str) -> bool {
    let branches = text
        .lines()
        .filter(|l| l.contains("├──") || l.contains("└──") || l.contains("`--"))
        .count();
    branches >= 6
}

fn looks_like_docker(text: &str) -> bool {
    let steps = text
        .lines()
        .filter(|l| l.trim_start().starts_with("Step ") && l.contains('/'))
        .count();
    steps >= 4 || (text.contains("\n#") && text.contains("DONE") && steps >= 2)
}

fn looks_like_bundler(text: &str) -> bool {
    let transforming = text
        .lines()
        .filter(|l| {
            let t = l.trim_start();
            t.starts_with("transforming ")
                || t.starts_with("rendering chunks")
                || t.starts_with("computing gzip")
                || t.contains("compiled successfully")
        })
        .count();
    transforming >= 3 || (text.contains("vite v") && text.contains("built in"))
}

fn reduce_install(text: &str) -> String {
    let mut summary = Vec::new();
    let mut errors = Vec::new();
    let mut warns = 0u32;
    let mut deprecated = 0u32;
    for line in text.lines() {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        let l = t.to_ascii_lowercase();
        if l.contains("http fetch")
            || l.contains("downloading ")
            || (l.contains("fetching ") && !is_diagnostic_line(t))
        {
            continue;
        }
        if l.contains("deprecated") {
            deprecated += 1;
            continue;
        }
        if is_diagnostic_line(t)
            || l.contains("err!")
            || l.contains("error ")
            || l.starts_with("error:")
            || l.contains("could not resolve")
            || l.contains("enoent")
            || l.contains("conflict")
        {
            if errors.len() < 16 {
                errors.push(t.to_string());
            }
            continue;
        }
        if l.starts_with("npm warn") || l.starts_with("warning ") || l.contains("warn ") {
            warns += 1;
            continue;
        }
        if summary.len() < 8
            && (l.contains("added ")
                || l.contains("audited ")
                || l.contains("done in")
                || l.contains("packages in")
                || l.contains("removed ")
                || l.contains("changed ")
                || l.starts_with("up to date")
                || (l.contains("downloaded ") && l.contains("crates")))
        {
            summary.push(t.to_string());
        }
    }
    let mut body = String::new();
    if summary.is_empty() && errors.is_empty() {
        body.push_str("install ok\n");
    }
    for s in &summary {
        body.push_str(s);
        body.push('\n');
    }
    if warns > 0 || deprecated > 0 {
        body.push_str(&format!("{warns} warnings, {deprecated} deprecated\n"));
    }
    if !errors.is_empty() {
        body.push('\n');
        for e in &errors {
            body.push_str(e);
            body.push('\n');
        }
    }
    body
}

fn reduce_tree(text: &str) -> String {
    let mut crates = 0u32;
    let mut kept = Vec::new();
    for line in text.lines() {
        let t = line.trim_end();
        if t.is_empty() {
            continue;
        }
        if t.contains("├──") || t.contains("└──") || t.contains("`--") || t.contains("│")
        {
            crates += 1;
            let depth = tree_depth(t);
            if depth <= 1 && kept.len() < 32 {
                kept.push(t.trim().to_string());
            }
            continue;
        }
        if crates == 0 && kept.len() < 4 {
            kept.push(t.to_string());
        }
    }
    let mut body = if crates > 0 {
        format!("{crates} crates\n")
    } else {
        String::new()
    };
    for k in &kept {
        body.push_str(k);
        body.push('\n');
    }
    if crates > kept.len() as u32 {
        body.push_str(&format!(
            "… {} deeper\n",
            crates.saturating_sub(kept.len() as u32)
        ));
    }
    if body.is_empty() {
        crate::compact::diagnostic_excerpt(text, 24)
    } else {
        body
    }
}

fn tree_depth(line: &str) -> u32 {
    if line.contains('│') {
        2
    } else {
        1
    }
}

fn reduce_docker(text: &str) -> String {
    let mut steps = Vec::new();
    let mut errors = Vec::new();
    let mut last_ok = None;
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with("Step ") || t.starts_with("#") && t.contains('[') {
            steps.push(t.to_string());
        }
        if errors.len() < 12
            && (t.starts_with("ERROR")
                || t.starts_with("error:")
                || t.contains("failed to")
                || is_diagnostic_line(t))
        {
            errors.push(t.to_string());
        }
        if t.contains("exporting to image")
            || t.contains("naming to")
            || t.starts_with("Successfully tagged")
            || t.contains("writing image")
        {
            last_ok = Some(t.to_string());
        }
    }
    let mut body = format!("{} steps\n", steps.len());
    let start = steps.len().saturating_sub(4);
    for s in &steps[start..] {
        body.push_str(s);
        body.push('\n');
    }
    if let Some(ok) = last_ok {
        body.push_str(&ok);
        body.push('\n');
    }
    if !errors.is_empty() {
        body.push('\n');
        for e in &errors {
            body.push_str(e);
            body.push('\n');
        }
    }
    body
}

fn reduce_bundler(text: &str) -> String {
    let mut summary = Vec::new();
    let mut errors = Vec::new();
    for line in text.lines() {
        let t = line.trim();
        let l = t.to_ascii_lowercase();
        if l.starts_with("transforming") || l.starts_with("rendering") || l.starts_with("computing")
        {
            continue;
        }
        if t.contains("error")
            || t.contains("ERROR")
            || is_diagnostic_line(t)
            || t.contains("failed to")
        {
            if errors.len() < 16 {
                errors.push(t.to_string());
            }
            continue;
        }
        if summary.len() < 12
            && (l.contains("built in")
                || l.contains("compiled")
                || l.contains("kB")
                || l.contains("dist/")
                || l.starts_with("vite v"))
        {
            summary.push(t.to_string());
        }
    }
    let mut body = String::new();
    for s in &summary {
        body.push_str(s);
        body.push('\n');
    }
    if body.is_empty() && errors.is_empty() {
        return compact_block(text, 24);
    }
    if !errors.is_empty() {
        body.push('\n');
        for e in &errors {
            body.push_str(e);
            body.push('\n');
        }
    }
    body
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tokens::estimate_tokens;

    #[test]
    fn npm_install_keeps_added_drops_fetch() {
        let mut raw = String::from("npm http fetch GET 200 https://registry.npmjs.org/lodash\n");
        for i in 0..40 {
            raw.push_str(&format!(
                "npm http fetch GET 200 https://registry.npmjs.org/pkg{i} 12ms\n"
            ));
        }
        raw.push_str("npm warn deprecated old@1.0.0: use new\n");
        raw.push_str("added 342 packages, and audited 343 packages in 12s\n");
        raw.push_str("npm ERR! code ERESOLVE\n");
        raw.push_str("npm ERR! could not resolve react@18\n");
        let out = reduce_install(&raw);
        assert!(out.contains("added 342"), "{out}");
        assert!(
            out.contains("ERESOLVE") || out.contains("could not resolve"),
            "{out}"
        );
        assert!(!out.contains("registry.npmjs.org/pkg12"), "{out}");
        assert!(estimate_tokens(&out) < estimate_tokens(&raw) / 4, "{out}");
    }

    #[test]
    fn cargo_tree_is_top_level() {
        let mut raw = String::from("ctx v0.2.0\n");
        raw.push_str("├── serde v1.0.0\n");
        raw.push_str("│   └── serde_derive v1.0.0\n");
        raw.push_str("│       └── syn v2.0.0\n");
        for i in 0..20 {
            raw.push_str(&format!("├── crate{i} v1.0.0\n"));
            raw.push_str(&format!("│   └── dep{i} v1.0.0\n"));
        }
        raw.push_str("└── tokio v1.0.0\n");
        let out = reduce_tree(&raw);
        assert!(out.contains("serde v1.0.0"), "{out}");
        assert!(out.contains("crates"), "{out}");
        assert!(!out.contains("serde_derive"), "{out}");
    }
}
