# Architecture

CTX is a **context runtime**, not a plugin. Harnesses dump tool output into the model. CTX stores the raw bytes locally and delivers a working set.

```text
Claude Code ──┐
Cursor ───────┤
Windsurf ─────┤  hooks / MCP / ctx exec
Continue ─────┼──────────►  ctx
JetBrains ────┤              │
VS Code ──────┤     ┌────────┼────────┐
Aider / Codex ┘     │        │        │
                    │   Store    Pager   Optimizer
                    │     │        │        │
                    │     └────────┼────────┘
                    │              │
                    └─────────►  MCP
                          ctx_fetch / ctx_read / ctx_search
```

## Principles

1. **Raw context is immutable.** Optimization never mutates stored bytes.
2. **Optimization must be reversible.** Every virtualized payload has a `ctx://` URI that restores the original.

No cloud. No extra API tokens. No LLM summarizer.

## Crates

| Crate | Job |
|---|---|
| `ctx-protocol` | Harness-agnostic events, `ctx://` URIs, tool kinds |
| `ctx-store` | SQLite + BLAKE3 + zstd. mmap blobs, WAL write + r2d2 `query_only` reads, FTS, bloom, snapshots |
| `ctx-optimizer` | Deterministic reducers (shell / file / mcp / duplicate / CoW) + WASM/command plugins |
| `ctx-pager` | HOT / WARM / COLD clock + TF-IDF mapped set + prefetch |
| `ctx-core` | Ingest, hooks, fetch, search |
| `ctx-mcp` | stdio MCP: `ctx_fetch`, `ctx_read`, `ctx_search`, `ctx_inspect`, `ctx_why` |
| `ctx-telemetry` | `ctx status` / `ctx why` snapshots |
| `apps/cli` | `ctx` binary, localhost dashboard, `ctx ci` |

Adapters convert Claude `PostToolUse` / Cursor `postToolUse` into one `CtxEvent`. Core does not know harness JSON.

## Store

```text
~/.ctx/          (or $CTX_HOME, or ./.ctx if home is not writable)
  ctx.db         schema v6 — pages, frames, sessions, observations, FTS5
  store/xx/yy.zst
  config.json
  snapshots/
  versions/
  bin/           aider-ctx wrapper
```

A page is not a blob. Ingest builds a **frame table** (failed tests, rustc errors, symbols) only when the payload is large enough (or is a file). `ctx://shell/abc#auth::login` is a virtual address: fetch walks the table. Search checks an in-memory bloom filter, then frames, then FTS. Identical payloads share a BLAKE3 page; whitespace-normalized near-dups increment a fingerprint.

`ingest()` commits blob + page + observation in one WAL transaction. Blobs are mmap'd on read. zstd level follows size and kind. Dashboard / MCP / search take connections from an r2d2 pool with `query_only=1`; ingest holds the write connection.

## Pager

Clock totals stay: **HOT** = referenced + recent; **WARM** = referenced or last 24h; **COLD** = old unreferenced. Referenced means error/fail, nonzero shell exit, or a path in `git diff`.

The **mapped page list** is ranked by task tokens (prompt, command, path, frame names), then IDF (rare tokens beat common ones) and fail/error frames. Overlap beats recency. Compiler spans prefetch `ctx://file/…#fn` without inlining the file. `rg`/`find` dumps fold to per-file samples. `ctx_fetch` with a line number opens the enclosing function. A COLD page from yesterday's Claude session can map into today's Cursor session — same store.

SessionStart greets with a tiny mapped set. Compact cannot inject Claude `additionalContext` (schema); PreCompact prints a plain-text keep list, and the next UserPromptSubmit remaps. Prompts contribute tokens only — they are never stored as blobs.

## Optimizers

| | Job |
|---|---|
| ShellGuard | Test/build/lint logs → summary + failures |
| ReadGuard | Unchanged / medium files → stub or outline |
| DuplicateGuard | Exact and whitespace-normalized repeats |
| MCPGuard | Schema-aware JSON reduction |
| GenericGuard | Head/tail + error lines |
| PluginGuard | User WASM (`optimize(i32)->i32`) or a 400ms stdin/stdout command |

`config.json` `optimizers` is a list of names (`"shell"`) or `{ "name", "path" }`. Empty = v0 pipeline. Plugins-only appends on top of v0. ABI example: `adapters/optimizer/identity.wat`.

Files above ~400 tokens become an outline + `ctx://`. Cursor large reads are denied and routed to `ctx_read`.

## Harnesses

**Claude Code:** `updatedToolOutput` replaces what the model sees. Shell, Read, MCP. `UserPromptSubmit` extracts task tokens.

**Cursor / Windsurf / Continue / Copilot / JetBrains / Codex:** MCP stdio `ctx mcp`. Cursor also rewrites shell to `ctx exec --shell -- '…'`. Native file/shell cannot be replaced in-place; large reads deny + `ctx_read`.

**Aider:** `ctx setup aider` writes `~/.ctx/bin/aider-ctx` → `ctx exec -- aider`.

`ctx setup --wizard` detects these, picks a budget, and can install the dashboard as a login service.

## Dashboard and ops

`ctx app` is a localhost SPA on `127.0.0.1:8741`: avoided-token KPIs, 7-day trend, optimizer split, HOT/WARM/COLD, config, pause/resume/snapshot. `/metrics` is Prometheus text.

`ctx snapshot` checkpoints SQLite. `ctx version pin|use|rollback` keeps copies under `~/.ctx/versions/`. `ctx uninstall --purge --yes` strips hooks and archives `~/.ctx`.

CI: `.github/actions/setup-ctx` plus `ctx ci --shell -- <cmd>` (markdown with `ctx://` links). A `v*` tag publishes GitHub Release binaries (macOS arm64/x86_64, Linux gnu/musl, Windows msvc) and `ghcr.io/<owner>/ctx`. Until that tag exists, `install.sh` and `cargo install --git` build from source.

## Non-goals

Cloud sync, team SaaS, model routing, extra API tokens, an Electron/Tauri shell. `ctx app` is a localhost page for today's avoided tokens — not a cloud dashboard. The moat is virtual-memory semantics, not a feature list.
