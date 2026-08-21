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

The upgrade path — cache-stable prefix, ledger-priced bills, intercept plane — is in [docs/plan/cache-stable-runtime.md](docs/plan/cache-stable-runtime.md).

No cloud. No extra API tokens. No LLM summarizer. Rust, local SQLite + BLAKE3 + zstd.

## Install

**macOS:** on [Releases](https://github.com/Echo888-cai/CTX/releases/latest) download **[CTX-Apple-Arm-v0.1.dmg](https://github.com/Echo888-cai/CTX/releases/download/v0.1/CTX-Apple-Arm-v0.1.dmg)** (Intel: `CTX-Apple-Intel-v0.1.dmg`). Open the disk image and **drag CTX.app into Applications**. The `-cli-` tarball is the command-line `ctx`, not the Mac app.

First launch, if macOS asks: Control-click CTX.app → Open.

CLI (macOS / Linux), which also installs `CTX.app` on Mac:

```bash
curl -fsSL https://raw.githubusercontent.com/Echo888-cai/CTX/main/install.sh | bash
```

Then:

```bash
ctx setup --wizard          # detect harnesses and pick a budget
ctx app                     # dashboard — today's tokens kept out of the model
ctx app --install-service   # optional: start the dashboard at login
```

Linux / Windows CLI: same [Releases](https://github.com/Echo888-cai/CTX/releases) page — `CTX-Linux-x64-v*.tar.gz`, `CTX-Linux-Arm-v*.tar.gz`, or `CTX-Windows-x64-v*.exe`.

The big number is **avoided tokens**. Raw context stayed on disk. `ctx status` is the same data in the terminal.

From source, if you prefer to build:

```bash
cargo install --git https://github.com/Echo888-cai/CTX --locked --force ctx-cli
ctx init
ctx setup --wizard
```

From a clone of this repo:

```bash
bash install.sh
# or
cargo install --path apps/cli --locked --force
ctx init
ctx setup all    # Claude, Cursor, Windsurf, VS Code, Continue, JetBrains, Aider, Codex
```

Docker from this tree (no registry required):

```bash
docker build -t ctx .
docker run --rm -v "$HOME/.ctx:/ctx" -e CTX_HOME=/ctx ctx status
```

Homebrew: `dist/homebrew/ctx.rb` points at the GitHub Release tarballs. Images: `ghcr.io/echo888-cai/ctx`.

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
| Deterministic reduction | Strip ANSI, passing tests, progress bars, git/npm/rg noise. Parsers, not a model. Plugins: WASM or a command. |
| Structural virtualization | Bytes go to a content-addressed store. Model gets a handle. |
| Semantic working set | Map pages by task tokens (TF-IDF). Cross-harness. Compact remaps. |

Details: [docs/architecture.md](docs/architecture.md)

## Harnesses

| | |
|---|---|
| **Claude Code** | Replaces tool output in place (`updatedToolOutput`). |
| **Cursor** | Wraps shell as `ctx exec`. MCP output replaced. Large files stay readable (fail-open). |
| **Windsurf** | MCP, same shape as Cursor. |
| **VS Code / Copilot** | Extension + user/workspace MCP. Status bar shows avoided tokens. |
| **Continue.dev** | `~/.continue/mcpServers/ctx.yaml` |
| **JetBrains AI** | MCP json for the IDE / `.idea`. |
| **Aider** | `~/.ctx/bin/aider-ctx` wraps `ctx exec -- aider`. |
| **Codex CLI** | `[mcp_servers.ctx]` in `~/.codex/config.toml`. |

```bash
ctx setup claude
ctx setup cursor
ctx setup vscode
ctx doctor
```

## Day two

```bash
ctx inspect --json          # HOT / WARM / COLD
ctx snapshot create
ctx version pin
ctx version rollback
ctx ci --shell -- cargo test
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
