[中文](README.zh.md)

# CTX

**Virtual memory for AI context.**

Your coding agent dumps everything it knows into the prompt. Test logs. File bodies. JSON blobs. The model drowns; you pay for the noise.

CTX sits in between. Raw output is stored on disk. The model gets a **working set** — the page it needs right now — plus a `ctx://` address to page the rest back in.

```text
cargo test
   18,241 tokens
        │
       CTX
        │
      412 tokens

raw bytes never leave your machine
```

Same task. Less noise. More room to think.

> Give AI less context. Give it the **right** context.

## Why it isn't another "token saver"

Most tools summarize with a small model, or drop the tail of the log. That's lossy compression.

CTX is **lossless virtualization**:

1. **Raw context is immutable.** Nothing in the store is rewritten.
2. **Every reduction is reversible.** `ctx://shell/abc#auth::login` walks a page table, not a guess.

No cloud. No extra API tokens. No LLM summarizer. Rust, local SQLite + BLAKE3 + zstd.

## Install

One line. Rust is installed if you don't have it.

```bash
curl -fsSL https://raw.githubusercontent.com/Echo888-cai/CTX/main/install.sh | bash
```

Then:

```bash
ctx app                 # dashboard — today's tokens kept out of the model
ctx app --install-service   # optional: start it at login
```

The big number is **avoided tokens**. Raw context stayed on disk. `ctx status` is the same data in the terminal.

From this repo:

```bash
bash install.sh
# or
cargo install --path apps/cli --locked --force
ctx init
ctx setup all    # Claude Code + Cursor
```

## What the model sees

```text
test auth::login ... FAILED
left: 401
right: 200

ctx://shell/9ba72f3c#auth::login  18241→412
```

Need the rest? `ctx_fetch` / `ctx_read` / `ctx_search`. The original log is still on disk.

Yesterday's Claude session and today's Cursor session share one page table. Pages are selected by **task**, not by recency of messages.

## How it works

```text
          Virtual Context
               200K
                │
         Context Store
                │
              Pager
                │
          Working Set
               31K
                │
               LLM
```

| Layer | What it does |
|---|---|
| Deterministic reduction | Strip ANSI, passing tests, progress bars, git/npm/rg noise. Parsers, not a model. |
| Structural virtualization | Bytes go to a content-addressed store. Model gets a handle. |
| Semantic working set | Map pages by task tokens. Cross-harness. Compact remaps. |

Details: [docs/architecture.md](docs/architecture.md)

## Harnesses

| | |
|---|---|
| **Claude Code** | Replaces tool output in place (`updatedToolOutput`). |
| **Cursor** | Wraps shell as `ctx exec`. MCP output replaced. Large files → `ctx_read`. |

```bash
ctx setup claude
ctx setup cursor
```

## Dogfood (this repo)

Not a vendor bench. One machine, this codebase:

| | Raw | Delivered |
|---|---:|---:|
| `cargo test` | ~1,199 | ~91 |
| live workspace | ~1,279 | ~146 (↓91%) |

We do not invent dollar savings. Subscriptions and list prices make that a lie. `ctx why` shows *which* tokens were kept out, and why.

## Out of scope

Cloud sync. Team SaaS. Model routing. Extra API tokens. An Electron/Tauri shell.

The local dashboard (`ctx app`) is just this machine's numbers on `127.0.0.1`.

Small, local, reversible.

## License

[MIT](LICENSE)
