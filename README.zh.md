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

无云、无额外 API token、不用 LLM 做摘要。Rust，本地 SQLite + BLAKE3 + zstd。

## 安装

一行。没有 Rust 会先装 rustup。

```bash
curl -fsSL https://raw.githubusercontent.com/Echo888-cai/CTX/main/install.sh | bash
```

然后：

```bash
ctx app                 # 仪表盘：今天挡在模型外面的 token
ctx app --install-service   # 可选：登录后自动开
```

最大那个数字就是 **少进模型的 token**。原文还在磁盘上。终端里同一份数据：`ctx status`。

在本仓库：

```bash
bash install.sh
# 或
cargo install --path apps/cli --locked --force
ctx init
ctx setup all    # Claude Code + Cursor
```

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
| 确定性削减 | 剥 ANSI、通过的测试、进度条。解析器，不是模型。 |
| 结构虚拟化 | 字节进内容寻址 store。模型拿到 handle。 |
| 语义 working set | 按任务 token 映射页。跨 harness。Compact 后重新映射。 |

细节：[docs/architecture.md](docs/architecture.md)

## 接入

| | |
|---|---|
| **Claude Code** | 原地替换工具输出（`updatedToolOutput`）。 |
| **Cursor** | shell 改写成 `ctx exec`。MCP 输出可替换。大文件 → `ctx_read`。 |

```bash
ctx setup claude
ctx setup cursor
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
