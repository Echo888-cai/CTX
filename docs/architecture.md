# Architecture

CTX is a **context runtime**, not a plugin. Harnesses dump tool output into the model. CTX stores the raw bytes locally and delivers a working set.

```text
Claude Code ──┐
              │  hooks
Cursor ───────┼──────────►  ctx
              │              │
              │     ┌────────┼────────┐
              │     │        │        │
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
| `ctx-store` | SQLite + BLAKE3 + zstd. Pages, frames, FTS, fingerprints |
| `ctx-optimizer` | Deterministic reducers (shell / file / mcp / duplicate / CoW) |
| `ctx-pager` | HOT / WARM / COLD clock + task-token mapped set |
| `ctx-core` | Ingest, hooks, fetch, search |
| `ctx-mcp` | stdio MCP: `ctx_fetch`, `ctx_read`, `ctx_search`, `ctx_inspect`, `ctx_why` |
| `ctx-telemetry` | `ctx status` / `ctx why` snapshots |
| `apps/cli` | `ctx` binary |

Adapters convert Claude `PostToolUse` / Cursor `postToolUse` into one `CtxEvent`. Core does not know harness JSON.

## Store

```text
~/.ctx/          (or $CTX_HOME, or ./.ctx if home is not writable)
  ctx.db         schema v6 — pages, frames, sessions, observations, FTS5
  store/xx/yy.zst
  config.json
```

A page is not a blob. Ingest builds a **frame table** (failed tests, rustc errors, symbols). `ctx://shell/abc#auth::login` is a virtual address: fetch walks the table. Search walks frames, then FTS. Identical payloads share a BLAKE3 page; whitespace-normalized near-dups increment a fingerprint.

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

Files above ~400 tokens become an outline + `ctx://`. Cursor large reads are denied and routed to `ctx_read`.

## Harnesses

**Claude Code:** `updatedToolOutput` replaces what the model sees. Shell, Read, MCP. `UserPromptSubmit` extracts task tokens.

**Cursor:** `preToolUse` rewrites shell to `ctx exec --shell -- '…'`. MCP via `updated_mcp_tool_output`. Native file/shell cannot be replaced in-place; large reads deny + `ctx_read`.

## Non-goals

Cloud sync, team SaaS, model routing, extra API tokens, an Electron/Tauri shell. `ctx app` is a localhost page for today's avoided tokens — not a cloud dashboard. The moat is virtual-memory semantics, not a feature list.
