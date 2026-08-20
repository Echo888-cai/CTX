# CTX = Cache-Stable Context Runtime

**Slogan:** CTX keeps the cache hot, the context cold, and the bill honest.

CTX is not a compressor. It is a cache-aware context I/O layer between the model and the outside world:

> Don't compress everything. Keep stable things cached. Keep large things local. Load relevant things on demand. Never destroy information. Compact only when economically rational.

This document is the upgrade plan. Stage 0 is implemented in the same change set as this file.

---

## Why this rewrite

Hook-mode CTX already stores raw bytes and delivers a working set. That is the right *data plane*. It is the wrong *product claim* if:

1. Page-in is dead (MCP does not speak the stdio spec) — reductions become irreversible.
2. Large Cursor reads are denied toward `ctx_read` that never arrives — fail-closed.
3. Near-duplicate folding hides numeric changes (`401` vs `200`).
4. An unwritable `~/.ctx` silently creates `./.ctx` — two ledgers, two configs.
5. Dashboard “savings” multiply a character heuristic by a single input list price, ignoring cache read 0.1×, cache write, output, thinking, and **how many later turns re-bill the same tokens**.

The north star is a real bill, not “98% tokens saved.”

---

## Two planes (do not mix them)

| | Observer plane (hooks, today) | Intercept plane (proxy, later) |
|---|---|---|
| Deploy | Zero extra URL. Every harness. | `ANTHROPIC_BASE_URL` / OpenAI-compatible proxy |
| Sees | Tool I/O, prompt submit, compact events | Full request body |
| Can change | Tool output, extra context | System, tools, order, `cache_control` |
| Cache work | Shrink the *tail*; delay compaction | Freeze L0/L1 prefix, canonicalize, Capability FS |

L0/L1 freeze, prompt canonicalization, and a 5-tool Capability FS **require the intercept plane**. Shipping them as hook theater would lie.

Honest hook-mode cache proposition:

> CTX cache value = delayed/avoided compaction + a smaller uncached tail each later turn.

Both are *measurable* from harness transcripts (Claude `cache_read_input_tokens`, Codex `cached_input_tokens` + `type: compacted`).

---

## Request shape (intercept plane)

```text
L0  CTX Protocol          — frozen for the epoch
L1  Workspace snapshot    — frozen for the epoch
        CACHE LINE
L2  Append-only journal
L3  Working set
L4  Current turn
```

Copy-on-Write overlays express file/state change. Cached prefix is never rewritten.

---

## Modules

| Module | Job |
|---|---|
| Cache Spine | Byte-stable system/tools/base prefix (intercept) |
| Context FS | CAS bytes, handles, versions, source maps |
| Overlay Engine | CoW deltas vs an immutable base snapshot |
| Working Set Manager | What the model is allowed to see now |
| Page Fault Engine | `ctx_fetch` / `ctx_read` / `ctx_search` — lossless recall |
| Execution Plane | AST, SQL, log parse, tool composition off the model |
| Cache Economics Engine | Compact / page-in / epoch rotation from **ledger** numbers |
| Ledger | Per-turn measured usage, model id, quota, cache TTL |

Moat = Ledger + counterfactual measurement + Cache Governor. Not a better gzip.

---

## Stages

### Stage 0 — Stop the bleeding (this change)

- MCP stdio: **NDJSON** (spec). Keep `Content-Length` as a reader-only compatibility mode.
- `initialize` echoes the client's `protocolVersion`.
- `ctx doctor` actually spawns `ctx mcp` and runs `initialize` + `tools/list`.
- Hooks **never deny** a tool. Large files ingest; page-in is opt-in via MCP.
- Near-duplicate Hamming default **0**. Status-code / digit-run changes never collapse.
- Unwritable home → **error** (`CTX_HOME=…`). No silent `./.ctx`. `ctx exec` already fail-opens.

### Stage 1 — Ledger ✓

`crates/ctx-ledger`. Parse on-disk usage, never invent a price for unknown models. `ctx ledger --sync`.

| Harness | Source | Cache fields |
|---|---|---|
| Claude Code | `~/.claude/projects/*/*.jsonl` | `cache_read_input_tokens`, `cache_creation.ephemeral_5m/1h` |
| Codex | `~/.codex/sessions/**/rollout-*.jsonl` | `cached_input_tokens`, `cache_write_input_tokens`, `compacted` |
| Cursor | `state.vscdb` `bubbleId:*` | `tokenCount` often zero — mark `partial` |

Model resolution: transcript model (`measured`) > hook `model_id` (`reported`) > config (`inferred`) > unknown (**no USD**).

Cost is not `avoided * input_price`. It is:

```text
Σ turns  uncached_input·P_in + cache_read·P_read + cache_write·P_write
       + output·P_out + thinking·P_think
+ P(avoided a compaction) × that compaction's measured cost
```

Subscribers get **quota**, not dollars (`used_percent`, `plan_type`, `resets_at`).

### Stage 2 — Cache Governor ✓

Per-turn hit rate from the ledger. Drift: model switch, 5m/1h TTL idle, compaction, prefix hash. `compact_advise` is injected on PreCompact; PostCompact rotates the epoch. Hook-mode cannot *deny* a provider compact.

### Stage 3 — Context FS v2 ✓

Base snapshot + overlay chain. Append-only delivery. Reliable page-in with refetch accounting. SessionStart opens an epoch; file hash change appends an overlay.

### Stage 4 — Intercept plane ✓

`ctx serve [--upstream URL] [--capability] [--dry-run]`. Injects L0+L1 into `system` with a cache breakpoint. Capability FS executes `ctx_search` / `ctx_fetch` / `ctx_inspect` / `ctx_apply` / `ctx_exec` (shell + indexed schemas). Foreign MCP tools are indexed, not auto-bound. Forwards `x-api-key` / `Authorization`. Hook-mode cannot freeze provider tool schemas — only this plane can.

### Stage 5 — Proof ✓

`ctx proof` splits live vs shadow observations. Compare delivered + ledger USD.

---

## Dashboard contract

Every number is `measured` or `estimated`. Never mix them in one KPI without a badge.

```text
RAW DATA / DATA KEPT LOCAL / MODEL WORKING SET     (estimated, CTX)
PROVIDER CACHE hit / write / miss                  (measured, ledger)
EFFECTIVE INPUT COST  or  QUOTA DELTA              (measured when priced)
PAGE FAULTS / RECALLS / RETRIES / TASK PASSED
```

---

## Non-goals (still)

Cloud sync, team SaaS, charging extra API tokens to “save tokens,” pretending hook-mode can freeze Anthropic tool definitions.
