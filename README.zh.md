[English](README.md)

# CTX

**AI 上下文的虚拟内存。**

编码 Agent 会把知道的一切倒进提示词。测试日志、整文件、JSON。模型被淹没，你为噪音付钱。

CTX 挡在中间。原文存在本地磁盘。模型只拿到此刻的 **working set**，外加一个 `ctx://` 地址，缺页时再调回来。

```text
cargo test
   18,241 tokens
        │
       CTX
        │
      412 tokens

原文不离开你的机器
```

同一件事。更少噪音。更多思考空间。

> Give AI less context. Give it the **right** context.

## 为什么不是又一个「省 token」插件

常见做法：用小模型做摘要，或砍掉日志尾巴。那是有损压缩。

CTX 是 **无损虚拟化**：

1. **原文不可变。** 存储里的字节从不被改写。
2. **每一次削减都可逆。** `ctx://shell/abc#auth::login` 走页表，不是猜。

升级路径（缓存稳定前缀、真实账本、拦截平面）见 [docs/plan/cache-stable-runtime.md](docs/plan/cache-stable-runtime.md)。

无云、无额外 API token、不用 LLM 做摘要。Rust，本地 SQLite + BLAKE3 + zstd。

## 安装

**macOS：** 在 [Releases](https://github.com/Echo888-cai/CTX/releases/latest) 下载 **[CTX-Apple-Arm-v0.1.4.dmg](https://github.com/Echo888-cai/CTX/releases/download/v0.1.4/CTX-Apple-Arm-v0.1.4.dmg)**（Intel 用 `CTX-Apple-Intel-v0.1.4.dmg`）。打开后运行 **Install CTX.command**，或把 **CTX.app** 拖进「应用程序」。打开 App 会自动接入已安装的 Cursor / Claude Code / ChatGPT；退出即暂停；删除 App 会还原那些配置。

若提示无法打开、来自身份不明的开发者：按住 Control 点 CTX.app → 打开。这是系统对未公证下载的隔离。下面这条命令行安装会去掉隔离。要彻底不弹窗，需要 Apple 开发者账号做公证。

命令行（macOS / Linux），Mac 上会顺便装好 `CTX.app`：

```bash
curl -fsSL https://raw.githubusercontent.com/Echo888-cai/CTX/main/install.sh | bash
```

然后：

```bash
ctx setup --wizard          # 探测 harness，选预算
ctx app                     # 仪表盘：今天挡在模型外面的 token
ctx app --install-service   # 可选：登录后自动开仪表盘
```

Linux / Windows 的 CLI 在同一页 [Releases](https://github.com/Echo888-cai/CTX/releases)：`CTX-Linux-x64-v*.tar.gz`、`CTX-Linux-Arm-v*.tar.gz` 或 `CTX-Windows-x64-v*.exe`。

最大那个数字就是 **少进模型的 token**。原文还在磁盘上。终端里同一份数据：`ctx status`。

从源码安装（备选）：

```bash
cargo install --git https://github.com/Echo888-cai/CTX --locked --force ctx-cli
ctx init
ctx setup --wizard
```

在本仓库：

```bash
bash install.sh
# 或
cargo install --path apps/cli --locked --force
ctx init
ctx setup all    # Claude、Cursor、Windsurf、VS Code、Continue、JetBrains、Aider、Codex
```

Docker 用这份代码构建（不依赖镜像仓库）：

```bash
docker build -t ctx .
docker run --rm -v "$HOME/.ctx:/ctx" -e CTX_HOME=/ctx ctx status
```

Homebrew：`dist/homebrew/ctx.rb` 指向 GitHub Release 的 tarball。镜像：`ghcr.io/echo888-cai/ctx`。

## 模型看到什么

```text
test auth::login ... FAILED
left: 401
right: 200

ctx://shell/9ba72f3c#auth::login  18241→412
```

需要剩下的？`ctx_fetch` / `ctx_read` / `ctx_search`。原文还在磁盘上。

昨天 Claude、今天 Cursor，同一张页表。按 **任务** 选页，不是按消息新旧。

## 怎么工作

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

| 层 | 做什么 |
|---|---|
| 确定性削减 | 剥 ANSI、通过的测试、进度条、git/npm/rg 噪音。解析器，不是模型。可挂 WASM / 命令插件。 |
| 结构虚拟化 | 字节进内容寻址 store。模型拿到 handle。 |
| 语义 working set | 按任务 token（TF-IDF）映射页。跨 harness。Compact 后重新映射。 |

细节：[docs/architecture.md](docs/architecture.md)

## 接入

| | |
|---|---|
| **Claude Code** | 原地替换工具输出（`updatedToolOutput`）。 |
| **Cursor** | shell 改写成 `ctx exec`。MCP 输出可替换。大文件保持可读（fail-open）。 |
| **Windsurf** | MCP，形状同 Cursor。 |
| **VS Code / Copilot** | 扩展 + 用户/工作区 MCP。状态栏显示少进模型的 token。 |
| **Continue.dev** | `~/.continue/mcpServers/ctx.yaml` |
| **JetBrains AI** | IDE / `.idea` 的 MCP json。 |
| **Aider** | `~/.ctx/bin/aider-ctx` 包装 `ctx exec -- aider`。 |
| **Codex CLI** | `~/.codex/config.toml` 里的 `[mcp_servers.ctx]`。 |

```bash
ctx setup claude
ctx setup cursor
ctx setup vscode
ctx doctor
```

## 第二天

```bash
ctx inspect --json          # HOT / WARM / COLD
ctx snapshot create
ctx version pin
ctx version rollback
ctx ci --shell -- cargo test
```

## 自测（本仓库）

不是厂商评测。一台机器，这份代码：

| | 原文 | 交给模型 |
|---|---:|---:|
| `cargo test` | ~1,199 | ~91 |
| live workspace | ~1,279 | ~146（↓91%） |

**不编美元。** 订阅和标价会让「省了多少钱」变成假账。`ctx why` 写明少了哪些 token、为什么。

## 不做

云同步、团队 SaaS、模型路由、额外 API token、Electron/Tauri 壳。

本地仪表盘（`ctx app`）只是这台机器上的数字，绑在 `127.0.0.1`。

小、本地、可逆。

## License

[MIT](LICENSE)
