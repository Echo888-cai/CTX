//! User-supplied optimizers: WASM modules or stdin/stdout executables.
//!
//! WASM ABI (single exported memory + `optimize`):
//! - Host writes the UTF-8 payload at offset 0.
//! - `optimize(len: i32) -> i32` returns the new length, also at offset 0.
//! - The plugin must not grow memory past 16 MiB.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::pipeline::{OptimizeInput, OptimizeOutput, Optimizer};
use crate::tokens::estimate_tokens;

const WASM_CAP: usize = 16 * 1024 * 1024;
const PLUGIN_TIMEOUT: Duration = Duration::from_millis(400);

pub struct PluginGuard {
    name: &'static str,
    path: PathBuf,
}

impl PluginGuard {
    pub fn new(name: &str, path: impl Into<PathBuf>) -> Self {
        Self {
            name: intern(name),
            path: path.into(),
        }
    }
}

impl Optimizer for PluginGuard {
    fn apply(&self, input: &OptimizeInput<'_>) -> Option<OptimizeOutput> {
        if input.raw_tokens < 80 {
            return None;
        }
        let text = if self.path.extension().and_then(|e| e.to_str()) == Some("wasm") {
            run_wasm(&self.path, input.payload).ok()?
        } else {
            run_exec(&self.path, input)?
        };
        if text.is_empty() || estimate_tokens(&text) + 40 >= input.raw_tokens {
            return None;
        }
        let mut out = OptimizeOutput::reduced(self.name, text);
        out.terminal = true;
        Some(out)
    }
}

fn intern(s: &str) -> &'static str {
    Box::leak(s.to_string().into_boxed_str())
}

fn run_exec(path: &Path, input: &OptimizeInput<'_>) -> Option<String> {
    let mut child = Command::new(path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let payload = serde_json::json!({
        "kind": input.kind,
        "tool_name": input.tool_name,
        "payload": input.payload,
        "raw_tokens": input.raw_tokens,
        "metadata": input.metadata,
    });
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(payload.to_string().as_bytes());
    }
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if started.elapsed() > PLUGIN_TIMEOUT => {
                let _ = child.kill();
                return None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
            Err(_) => return None,
        }
    }
    let out = child.wait_with_output().ok()?;
    if !out.status.success() {
        return None;
    }
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
    v.get("text")
        .and_then(|t| t.as_str())
        .map(str::to_string)
        .filter(|s| !s.is_empty())
}

fn run_wasm(path: &Path, payload: &str) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    run_wasm_bytes(&bytes, payload)
}

pub fn run_wasm_bytes(wasm: &[u8], payload: &str) -> Result<String, String> {
    let raw = payload.as_bytes();
    if raw.len() + 64 > WASM_CAP {
        return Err("payload too large for plugin memory".into());
    }
    let engine = wasmi::Engine::default();
    let module = wasmi::Module::new(&engine, wasm).map_err(|e| e.to_string())?;
    let mut store = wasmi::Store::new(&engine, ());
    let instance = wasmi::Linker::new(&engine)
        .instantiate(&mut store, &module)
        .map_err(|e| e.to_string())?
        .start(&mut store)
        .map_err(|e| e.to_string())?;
    let memory = instance
        .get_memory(&store, "memory")
        .ok_or_else(|| "wasm plugin must export memory".to_string())?;
    let needed = raw.len() as u32;
    memory
        .write(&mut store, 0, raw)
        .map_err(|e| e.to_string())?;
    let optimize = instance
        .get_typed_func::<i32, i32>(&store, "optimize")
        .map_err(|e| e.to_string())?;
    let new_len = optimize
        .call(&mut store, needed as i32)
        .map_err(|e| e.to_string())?;
    if new_len < 0 || new_len as usize > WASM_CAP {
        return Err("plugin returned an invalid length".into());
    }
    let mut out = vec![0u8; new_len as usize];
    memory
        .read(&store, 0, &mut out)
        .map_err(|e| e.to_string())?;
    String::from_utf8(out).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wasm_echo_shortens_to_ok() {
        let wasm = wat::parse_str(
            r#"
            (module
              (memory (export "memory") 1)
              (func (export "optimize") (param i32) (result i32)
                (i32.store8 (i32.const 0) (i32.const 111))
                (i32.store8 (i32.const 1) (i32.const 107))
                (i32.store8 (i32.const 2) (i32.const 10))
                i32.const 3))
            "#,
        )
        .unwrap();
        let out = run_wasm_bytes(&wasm, &"noise\n".repeat(80)).unwrap();
        assert_eq!(out, "ok\n");
    }
}
