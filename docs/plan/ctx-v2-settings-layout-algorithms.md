# CTX v2 方案：配色 / 布局 / 设置页 / 算法

面向实现者（Grok）的详细规格。所有文件路径、函数名、字段名均以当前仓库为准，可直接照做。
动手前先跑一次 `cargo test` 确认基线是绿的（上一轮改动收尾时全绿）。

---

## 0. 问题诊断（带证据）

| # | 问题 | 证据 |
|---|---|---|
| 1 | 下半部分布局别扭 | `apps/cli/src/app.html` 的 `.content` 是两列网格：左列**堆了两个 section**（上下文构成 + 上下文趋势），右列是 `.models { min-height: 467px }` 的表格，实际只有 3 行数据 → 右侧大片空白。 |
| 2 | 列中列 | 「上下文构成」的 legend 自身是 `grid-template-columns: 1fr 1fr`，又嵌在两列布局的左列里，形成列中列，视觉噪音大。 |
| 3 | KPI 不等距 | `.metric:nth-child(2)/(3)/(4)` 分别硬编码 `padding-left: 58px / 47px / 38px`，四张卡视觉重心不一致。 |
| 4 | 标签语义重复 | 第 2、3 张卡的 `metric-label` 都是「已节省」（一个是 tokens，一个是美元）。 |
| 5 | 配色语义反了 | 现在绿色 = 有效输入（delivered），灰色 = 已节省。用户预期：**绿 = 省下来的，蓝 = 该有的**。 |
| 6 | 不知道哪个工具开着 | `Config` 只有一个全局 `enabled`，**没有任何 per-harness 开关**（`crates/ctx-core/src/config.rs`）。`doctor::collect()` 能查出每个工具的接入状态，但**没有任何 HTTP 端点暴露给仪表盘**（`apps/cli/src/app.rs` 的 `dispatch_with` 里没有 `/api/doctor`、`/api/harnesses`）。 |
| 7 | 不知道「是否每次都省了」 | 后端 `status --json` 已经返回 `by_harness`（`apps/cli/src/status.rs:31`），但 Web 仪表盘完全没展示。而且 MCP-only 的工具（Codex/Windsurf/VS Code…）**根本不会自动省**，UI 里没有任何地方说明这件事。 |
| 8 | Cursor 钩子注册不全 | `setup.rs::merge_cursor_hooks` 只注册 6 个事件（`sessionStart/sessionEnd/preToolUse/postToolUse/beforeReadFile/beforeSubmitPrompt`），但 `hooks.rs::handle_hook_inner` 已经实现了 `afterShellExecution`、`afterMCPExecution`、`preCompact`、`postCompact` 的处理逻辑 —— 写了却没接线。 |
| 9 | Claude Desktop 完全没支持 | 仓库里搜不到 `claude_desktop_config.json`。`setup_claude()` 只写 `~/.claude/settings.json`（那是 CLI）。 |
| 10 | 节省数字没扣回取 | `runtime.rs::finish` 里 `avoided = raw_tokens - delivered_tokens`，之后 agent 用 `ctx_fetch` 把内容页回来的 token **没有从节省里扣掉**（`touch_referenced` 只置了标记位）。 |

---

## Part A：配色系统

### A1. 语义规则（先定死，后面所有图形都照这个来）

- **浅蓝 = 该有的**：`delivered` / 有效输入 / 真正进模型的上下文。
- **浅绿 = 省下的**：`avoided` / 各类优化削减。子类别用**同一绿色系的深浅阶梯**区分，保证"一眼看出这一整段都是省下来的"。
- **品牌绿 `#078b45` 从此只用于文字**（大号 KPI 数字、状态点、链接强调），**不再用于任何图形填充**，避免和"节省绿"混淆。
- 灰色只用于网格线、次要文字、禁用态。

### A2. 变量（写进 `apps/cli/src/app.html` 的 `:root`）

```css
:root {
  /* 既有的保留 */
  --paper: #ffffff; --ink: #111311; --muted: #6e736f; --quiet: #969b97;
  --line: #e4e7e4; --grid: #e9ece9; --wash: #f8f9f8; --green: #078b45;

  /* 新增：该有的（浅蓝） */
  --kept-50:  #eff5fd;
  --kept-100: #dce9fa;
  --kept-300: #a8c8ef;
  --kept-400: #7faee8;   /* 主色：构成条 / 柱底 */
  --kept-600: #4a83ce;   /* hover / 强调 */
  --kept-text:#2f6fbf;   /* 蓝色数字 */

  /* 新增：省下的（浅绿） */
  --save-50:  #edf7f1;
  --save-100: #d8f1e3;
  --save-300: #a3dcbe;
  --save-400: #86cfa8;   /* 主色：柱顶 */
  --save-600: #4fa97b;   /* hover / 强调 */
  --save-text:#2f8f5f;
}
```

### A3. 节省分类色阶（按 tokens 从大到小取用，越大越深）

```js
const SAVE_RAMP = ["#4fa97b", "#6cbb92", "#86cfa8", "#a3dcbe", "#bfe8d2", "#d8f1e3", "#eaf7ef"];
const KEPT_COLOR = "var(--kept-400)";
```

### A4. 要改的具体位置

| 文件 | 位置 | 改法 |
|---|---|---|
| `app.html` | `renderComposition` 里的 `COMP_COLORS` | 换成 `SAVE_RAMP`；`row.kept` 的段用 `KEPT_COLOR` |
| `app.html` | `.col-delivered { fill: var(--green) }` | → `fill: var(--kept-400)` |
| `app.html` | `.col-avoided { fill: #c9d0d6 }` | → `fill: var(--save-400)` |
| `app.html` | `.legend i` | 两个图例点分别用 `--kept-400`（有效输入）/`--save-400`（已节省） |
| `app.html` | `.comp-legend li.kept .tokens { color: var(--green) }` | → `color: var(--kept-text)`；非 kept 行的 tokens 用 `var(--save-text)` |
| `app.html` | `.metric-value`（大数字） | 保持 `var(--green)`（文字型品牌绿，不冲突） |
| `app.html` | `.saved` 表格列 | → `var(--save-text)` |
| `apps/macos/Sources/PopoverView.swift` | `segmentColor(row:index:)` | `row.kept` → `Color(hex: 0x7FAEE8)`；否则用与 `SAVE_RAMP` 一致的 7 色数组 |
| `apps/macos/Sources/PopoverView.swift` | 「有效输入 xxx」文字 | 从 `green` 改为 `Color(hex: 0x2F6FBF)` |

**验收**：构成条从左到右是「一段浅蓝 + 若干层次递减的浅绿」；趋势柱是浅蓝底 + 浅绿顶；两处 legend 色块与之一致；macOS 卡片同款。

---

## Part B：下半部分布局重构

### B1. 新的信息架构（全宽自上而下，取消"左右分栏塞不同主题"）

```
┌ 顶栏  brand | slogan            状态chip  模型▾  日期▾  │ ⚙
├ KPI 带（4 张等宽卡，每张 = 主数字 + 副行说明）
├ 上下文构成（全宽横条 + 自适应多列 legend）
├ 趋势（8 栏）        │ 按来源 / 工具（4 栏）
├ 明细（全宽，Tab：按模型 / 按工具 / 最近命中）
└ 页脚
```

### B2. 栅格与间距（写死的设计 token）

- 容器：`width: min(1180px, 100% - 72px)`（不变）
- 栅格：`display: grid; grid-template-columns: repeat(12, 1fr); column-gap: 24px`
- 区块纵向间距：**40px**（区块之间），区块内标题到内容 **18px**
- 间距刻度只允许用 4 / 8 / 12 / 18 / 24 / 40
- 分隔：不用卡片边框，用 `border-top: 1px solid var(--line)` 分区（延续现在的白纸风格）

### B3. KPI 带（替换现在的 `.results`）

四张卡等宽，**去掉所有硬编码 padding**：

```css
.kpis { display: grid; grid-template-columns: repeat(4, 1fr); gap: 0; padding: 30px 0 32px; }
.kpi { min-width: 0; padding: 0 28px; }
.kpi:first-child { padding-left: 0; }
.kpi:last-child { padding-right: 0; }
.kpi + .kpi { border-left: 1px solid var(--line); }
.kpi-value { color: var(--green); font-size: 56px; font-weight: 500; letter-spacing: -.02em; font-variant-numeric: tabular-nums; }
.kpi-value small { margin-left: 3px; font-size: 26px; }
.kpi-label { margin-top: 14px; color: #272a27; font-size: 16px; }
.kpi-sub { margin-top: 6px; color: var(--muted); font-size: 12px; line-height: 1.5; }
```

四张卡的内容（**每张都有副行，解决"两个已节省"的歧义**）：

| # | 主数字 | 标签 | 副行 |
|---|---|---|---|
| 1 | `63%` | 上下文优化率 | `原文 601.8K → 有效输入 221.9K`（用 → 而不是单独一张 transform 卡） |
| 2 | `380.0K` | 已节省 tokens | `净节省 365.2K · 回取 14.8K`（见 Part D1；没有净额数据时退化成 `本周 380.0K`） |
| 3 | `$0.76` +「估」 | 已省成本 | `按 grok-4.6 $2/M 估算`（非估算时显示 `官网价 · 3 个模型`） |
| 4 | `2/9` | 自动优化中 | `Cursor · Claude Code 已接钩子；7 个工具仅检索`（点击 = 打开设置抽屉的工具分区） |

第 4 张卡是**直接回答"我不知道哪里开着"**的入口，必须有，且 `cursor: pointer`。

### B4. 上下文构成（全宽，解决"列中列"）

```css
.composition { padding: 34px 0 36px; border-top: 1px solid var(--line); }
.comp-bar { display: flex; gap: 2px; height: 14px; border-radius: 7px; overflow: hidden; background: var(--wash); }
.comp-seg { height: 100%; min-width: 3px; transition: flex-grow .3s ease; }
.comp-legend {
  display: grid; grid-template-columns: repeat(auto-fill, minmax(210px, 1fr));
  gap: 2px 28px; margin: 18px 0 0; padding: 0; list-style: none;
}
.comp-legend li { display: grid; grid-template-columns: 10px minmax(0,1fr) auto 44px; align-items: center; gap: 10px; height: 30px; font-size: 13px; }
```

`auto-fill` 让 legend 在 1180px 下自然排 4 列、窄屏排 2 列 / 1 列，不再是死的两列。
构成条**必须带 hover tooltip**（复用 Part B6 的 `.tip`）：显示 `类别 / tokens / 占原文比例`。

### B5. 趋势 + 按来源（8 : 4）

```css
.row-trend { display: grid; grid-template-columns: 8fr 4fr; column-gap: 40px; padding: 34px 0 36px; border-top: 1px solid var(--line); }
.by-source { align-self: start; }           /* 关键：不要 min-height，让它按内容收缩 */
```

右侧「按来源」用 `by_harness`（后端已有，见 `status.rs:31`；仪表盘侧需要在 `status::dashboard()` 里同样返回一份 `by_harness`，按当前时间窗与模型筛选过滤）。每行：

```
Cursor            自动优化   276.9K   ▓▓▓▓▓▓▓░░░ 63%
Claude Code       自动优化    98.2K   ▓▓▓▓░░░░░░ 41%
Codex             仅检索         —    未接钩子
```

- 「自动优化 / 仅检索」是 chip：`.cap-hook { background: var(--save-50); color: var(--save-text) }`、`.cap-mcp { background: var(--wash); color: var(--muted) }`
- 进度条：底 `var(--kept-100)`，填充 `var(--save-400)`，高 6px 圆角 3
- 未接钩子的行整行 `opacity: .55`，右侧显示「接入」文字按钮 → `POST /api/setup?target=`

### B6. 明细（全宽 Tab 表）

替换现在孤零零的「按模型」表：

```html
<section class="detail">
  <div class="section-head">
    <div class="tabs" id="detail-tabs" role="tablist">
      <button role="tab" data-tab="model" class="on">按模型</button>
      <button role="tab" data-tab="tool">按工具</button>
      <button role="tab" data-tab="recent">最近命中</button>
    </div>
    <span class="range-label" id="detail-meta">—</span>
  </div>
  <table class="detail-table"><thead id="detail-head"></thead><tbody id="detail-rows"></tbody></table>
  <div class="detail-total"><span id="detail-count">共 0 项</span><b id="detail-sum">—</b></div>
</section>
```

- 行高统一 **52px**（现在是 66px，太松），最多显示 8 行，超出显示「查看全部 N 项」展开
- `.tabs button` 复用 `.chart-modes` 的胶囊样式（同一个组件，别再造一套）
- 「按工具」用 `by_harness`；「最近命中」用已有的 `recent`（`feed_rows`）

### B7. 响应式

- `< 1180px`：`.row-trend` 变 1 列；`.kpis` 变 2×2（`.kpi + .kpi` 的左边框改成用 `:nth-child(2n)` 判断）
- `< 760px`：KPI 变 1 列；legend 单列；`.detail-table` 横向滚动（`min-width: 420px`）

**验收**：1180px、1024px、768px、375px 四个宽度下都没有大块空白、没有横向溢出、没有嵌套多列 legend。

---

## Part C：设置页

### C1. 齿轮放哪（结论：顶栏最右，与筛选组用分隔线隔开）

```html
<div class="controls">
  <span class="status" id="status">…</span>
  <div class="ctl" id="model-wrap">…</div>
  <div class="ctl" id="range-wrap">…</div>
  <span class="ctl-sep" aria-hidden="true"></span>
  <button type="button" class="control icon-btn" id="settings-btn" aria-label="设置" aria-haspopup="dialog">
    <svg class="icon" width="16" height="16" viewBox="0 0 16 16">…齿轮…</svg>
  </button>
</div>
```

```css
.ctl-sep { width: 1px; height: 20px; margin: 0 4px; background: var(--line); }
.control.icon-btn { width: 36px; padding: 0; justify-content: center; }
.control.icon-btn .icon { color: var(--muted); transition: transform .35s ease, color .2s ease; }
.control.icon-btn:hover .icon { color: var(--ink); transform: rotate(35deg); }
```

理由：筛选控件回答"看什么"，设置回答"怎么工作"，两类不能混在同一组里；最右侧是"全局操作"的常规位置；36×36 与其它控件同高，不破坏刚统一好的顶栏节奏。

### C2. 形态（结论：右侧抽屉，不是新页面）

- `position: fixed; inset: 0 0 0 auto; width: 480px;` 全高，`box-shadow: -18px 0 48px rgba(17,19,17,.14)`
- 遮罩 `rgba(17,19,17,.28)`，点击遮罩 / `Esc` 关闭；`role="dialog" aria-modal="true"`，打开时焦点移进抽屉、关闭后还给齿轮
- 进出动画 `transform: translateX(100%) → 0`，`.22s cubic-bezier(.2,.8,.2,1)`
- `< 640px` 时 `width: 100%`
- 不做前端路由（当前是单文件内嵌页面，加路由不值当）

### C3. 抽屉分区

**① 运行状态**
- 大开关（`.switch`，见 C6）：`enabled` → `POST /api/pause` / `POST /api/resume`
- 说明文案：「暂停后所有钩子直通放行，不再优化，也不再计数。」
- 一行实时数字：`今日 1240 次拦截 · 已节省 380.0K · 平均耗时 8ms`。耗时不用新造轮子：`crates/ctx-store/src/observe.rs` 已经在输出 `ctx_hook_latency_seconds{quantile="0.5"|"0.9"|"0.99"}` 和 `_count`，`GET /api/health` 直接读同一组 `Samples` 即可
- **影子模式**开关（见 Part D2）：「只统计不改写 —— 用来验证 CTX 到底省了多少，但不影响 agent 收到的内容。」

**② 接入的工具**（回答"哪里开着 / 是不是每次都省"的核心）

分两组，组标题 + 一句话解释：

> **自动优化（已装钩子）** — 每次工具输出都会被 CTX 拦下来压缩，省的是真金白银。
> **仅检索（只装了 MCP）** — CTX 只提供 `ctx_search` / `ctx_fetch` 工具，**不会自动省 token**；只有模型主动调用才有用。

每行（`.harness-row`）：

```
[图标] Cursor                     自动优化      ●━━  [修复] 
       Desktop · CLI 共用 ~/.cursor/hooks.json
       今日 276.9K · ↓63%
```

- 左：名称 + 第二行灰色小字（形态 + 配置文件路径）+ 第三行本工具今日战绩
- 中：能力 chip（自动优化 / 仅检索）
- 右：开关 + 操作按钮（未接入→「接入」；已接入但二进制路径失效→「修复」；已接入→「移除」放在 hover 的更多菜单里，避免误点）
- 未安装该工具（本机检测不到）的行默认折叠进「显示未安装的 6 个工具」

**工具目录（必须按这张表实现，这是用户点名要的 desktop / cli 区分）**

| id | 显示名 | 形态 | 接入方式 | 配置文件 | 能力 | 备注 |
|---|---|---|---|---|---|---|
| `cursor` | Cursor | Desktop + CLI | hooks + MCP | `~/.cursor/hooks.json`、`~/.cursor/mcp.json` | 自动优化 | 用户级配置，Desktop 与 `cursor-agent` CLI 都读它。**实现前先验证一次**：装好钩子后在 CLI 里跑一个命令，看 `sessions` 表是否新增 cursor 会话；若 CLI 不读用户级配置，则单独列一行 `cursor-cli` |
| `claude-code` | Claude Code | CLI | hooks + MCP | `~/.claude/settings.json`、`~/.claude.json` | 自动优化 | |
| `claude-desktop` | Claude Desktop | Desktop | 仅 MCP | `~/Library/Application Support/Claude/claude_desktop_config.json` | 仅检索 | **当前完全没实现，需新增**（见 C5） |
| `codex` | Codex CLI | CLI | 仅 MCP | `~/.codex/config.toml` | 仅检索 | 已有 `setup_codex()` |
| `windsurf` | Windsurf | Desktop | 仅 MCP | `~/.codeium/windsurf/mcp_config.json` | 仅检索 | |
| `vscode` | VS Code | Desktop | 仅 MCP | `~/Library/Application Support/Code/User/mcp.json` 等 | 仅检索 | |
| `continue` | Continue.dev | 插件 | 仅 MCP | `~/.continue/config.yaml` | 仅检索 | |
| `jetbrains` | JetBrains | Desktop | 仅 MCP | JetBrains 配置目录 | 仅检索 | |
| `aider` | Aider | CLI | wrapper | — | 仅检索 | |
| `copilot` | GitHub Copilot | 插件 | 仅 MCP | — | 仅检索 |

**⚠️ Cursor 钩子有三个层级，必须处理（当前实现只认一个）**

Cursor 官方文档：`hooks.json` 可以存在于三个位置，**所有匹配的钩子都会运行**，不是"就近取一个"：

| 层级 | 路径 | CTX 现状 |
|---|---|---|
| 系统级 | macOS `/Library/Application Support/Cursor/hooks.json`（Linux `/etc/cursor/hooks.json`） | 不读不写 |
| 项目级 | `<repo>/.cursor/hooks.json` | 不读不写 |
| 用户级 | `~/.cursor/hooks.json` | ✅ setup 只写这一个 |

由此产生两件必须做的事：

1. **双计风险**：如果某个仓库的 `.cursor/hooks.json` 里也有 ctx（比如别人提交进 repo 的），同一个事件会触发**两次** ctx hook → 同一份输出被记两条 observation，节省数字翻倍。

   做法：`observations` 加 `dedup_key TEXT NOT NULL DEFAULT ''` + `CREATE UNIQUE INDEX idx_obs_dedup ON observations(dedup_key) WHERE dedup_key != ''`（部分索引，老数据的空 key 不受约束）。key 的构造（`hooks.rs` 里现在**一个调用 id 都没取**，需要补）：

   ```
   dedup_key = blake3(session_id | hook_event_name | call_id | content_hash)
   call_id = value["generation_id"]            // Cursor
           ?? value["tool_use_id"]             // Claude Code
           ?? value["tool_call_id"]
           ?? ""                               // 兜底：靠 content_hash + 秒级 created_at 区分
   ```

   插入走 `INSERT OR IGNORE`，受影响行数为 0 时 `IngestResult.deduped = true`，**不计入节省、不写 reasons**，但仍返回已缓存的 delivered 文本（保证第二次触发的 agent 也拿到压缩结果）。
   **这是数据可信度问题，优先级等同 D1** —— 净节省算在错的基数上没有意义。
2. **设置页要显示注册层级**：harness 行的第二行灰字里写清 `用户级 ~/.cursor/hooks.json`，并在检测到项目级/系统级也有 ctx 时给一条黄色提示「检测到项目级钩子，可能重复计数 —— 建议移除其中一处」。`/api/harnesses` 的响应里加 `registered_levels: ["user", "project"]`。
3. 顺带提示：云端 agent 不加载用户级钩子，只加载项目级 —— 设置页可以提一句「云端 agent 需要项目级钩子」，但本期不实现自动写入项目级配置。

**③ 优化强度**
- 分段控件：激进 / 均衡 / 保守 → `budget_strategy`（`extreme|balanced|conservative`）
- 两个数字输入：`virtualize_threshold_tokens`（默认 200，「低于这个体量直接放行」）、`large_file_tokens`（默认 400，「超过这个体量的文件读改走大纲 + ctx_read」）
- 每档下面写清后果，例如「激进：预算 ×0.72，省得更多，回取率可能上升」

**④ 计价**
- `default_billing_model` 下拉（选项来自 `GET /api/status` 的 `price_catalog`）+ 「留空 = 按 Grok 4.6 $2/M」
- 一行：`官网价已同步 155 条 · 2026-08-20 10:01` + 「立即刷新」按钮 → `POST /api/prices/refresh`

**⑤ 数据与快照**
- 库大小 / 页数 / 观测数（`pages`、`store_bytes`）
- 「新建快照」→ `POST /api/snapshot`；快照列表 + 「恢复」→ `POST /api/snapshot/restore?id=`
- `auto_snapshot`、`dashboard_autostart` 两个开关

**⑥ 高级**
- `optimizers` 列表（只读展示当前启用的 guard 名 + 各自今日贡献；为空则显示「默认流水线」）
- `enable_semantic`（当前是保留字段，显示为「敬请期待」的禁用开关）
- 诊断：把 `doctor::collect()` 的每条 check 渲染成 `✓/✗ 名称 — detail`
- 底部：版本号、「卸载 CTX」（二次确认，调 `uninstall`）

### C4. 后端 API 契约（全部加在 `apps/cli/src/app.rs::dispatch_with`）

```
GET  /api/harnesses
→ { "ok": true, "harnesses": [ {
      "id": "cursor", "name": "Cursor", "form": "desktop+cli",
      "integration": "hooks",            // hooks | mcp | wrapper
      "capability": "auto",              // auto | retrieval
      "detected": true,                  // 本机是否装了这个工具
      "installed": true,                 // ctx 是否已接入
      "stale": false,                    // 钩子里的二进制路径是否失效
      "enabled": true,                   // ctx 是否对它生效（Config.disabled_harnesses 取反）
      "shared_with": ["cursor-cli"],
      "config_paths": ["~/.cursor/hooks.json", "~/.cursor/mcp.json"],
      "today": { "raw": 601843, "delivered": 221903, "avoided": 379940, "reduction_pct": 63 }
  } ] }

POST /api/harness?id=cursor&enabled=0     → { ok, id, enabled }
POST /api/setup?target=cursor              → 已有
POST /api/uninstall?target=cursor          → 新增：只摘掉这一个工具的 ctx 条目
GET  /api/doctor                           → { ok, checks: [ { ok, name, detail } ] }
POST /api/prices/refresh                   → { ok, entries, fetched_at }
GET  /api/health                           → { ok, enabled, hook_p50_ms, hook_p95_ms, intercepts_today, shadow }
```

实现要点：
- `/api/harnesses` 的 `detected` 复用 `setup.rs` 里现成的 `detect_*()`（把它们改成 `pub(crate)`）；`installed`/`stale` 复用 `doctor.rs` 的 `hooks_contain_ctx` / `first_stale_hook_bin` / `mcp_registered`（已 `pub`）
- `today` 复用 `Snapshot::capture().by_harness_today`，按 `Harness::as_str()` 对齐 id（注意 `claude-code` vs `claude`：`harness.rs` 里 `Harness::ClaudeCode.as_str()` 是 `claude-code`，doctor 的 check name 是 `claude`，**必须做一层 id 映射表**，别直接字符串比对）
- 所有写操作都要 `ok/error` 包装，前端只认 `ok`

### C5. Config 结构变更（`crates/ctx-core/src/config.rs`）

```rust
/// 被用户关掉的 harness id（Harness::as_str() 值）。空 = 全开。
#[serde(default, skip_serializing_if = "Vec::is_empty")]
pub disabled_harnesses: Vec<String>,

/// 影子模式：照常统计，但把原文原样交给模型。
#[serde(default)]
pub shadow_mode: bool,

/// 只对这些 harness 开影子模式；为空且 shadow_mode=true 则全局生效。
#[serde(default, skip_serializing_if = "Vec::is_empty")]
pub shadow_harnesses: Vec<String>,
```

**钩子侧强制（`crates/ctx-core/src/hooks.rs::handle_hook_inner`，在 `detect_harness` 之后立刻做）**：

```rust
let harness = detect_harness(value);
if !runtime.config.enabled || runtime.is_harness_disabled(harness) {
    // 直通：按 harness 返回对应的空放行响应，绝不报错（fail-open 不变）
    return Ok(passthrough(event_name, harness));
}
```

`passthrough()` 要按事件类型返回正确的空响应形状：`preToolUse` → `{"permission":"allow"}`；Claude 系 → `{"hookSpecificOutput":{...}}` 空壳；其余 → 空 stdout。**这里写错会让 agent 卡住，必须有测试覆盖每个事件名。**

新增 `Runtime::is_harness_disabled(&self, h: Harness) -> bool`。

同时新增 Claude Desktop 支持：`setup.rs` 加 `detect_claude_desktop()`（判断 `~/Library/Application Support/Claude/` 是否存在）+ `setup_claude_desktop()`（往 `claude_desktop_config.json` 的 `mcpServers.ctx` 合并，复用现成的 `merge_mcp`），并接进 `setup("claude-desktop")`、`init()`、`doctor::collect()`、`uninstall`。

### C6. 开关组件（一个就够，别造第二套）

```css
.switch { position: relative; width: 40px; height: 22px; flex: 0 0 auto; }
.switch input { position: absolute; inset: 0; opacity: 0; margin: 0; cursor: pointer; }
.switch i { position: absolute; inset: 0; border-radius: 999px; background: #d7dbd7; transition: background .2s ease; }
.switch i::after { content: ""; position: absolute; top: 2px; left: 2px; width: 18px; height: 18px; border-radius: 50%; background: #fff; box-shadow: 0 1px 2px rgba(17,19,17,.2); transition: transform .2s ease; }
.switch input:checked + i { background: var(--save-600); }
.switch input:checked + i::after { transform: translateX(18px); }
.switch input:focus-visible + i { box-shadow: 0 0 0 3px rgba(79,169,123,.28); }
```

### C7. 前端状态与刷新

- 抽屉打开时拉一次 `/api/harnesses` + `/api/config` + `/api/doctor`（并行），关闭后不再轮询
- 任何写操作成功后：局部更新该行 + 触发一次主仪表盘 `refresh()`（因为开关会影响 KPI 第 4 张卡）
- 主仪表盘 2 秒轮询在抽屉打开时**暂停**，避免焦点被抢

---

## Part D：算法优化（按 ROI 排序）

### D1. 净节省会计：把"回取"扣掉 ★最高优先

**问题**：`crates/ctx-core/src/runtime.rs::finish` 里 `avoided = raw_tokens - delivered_tokens` 就落库了。之后模型用 `ctx_fetch` 把这一页原文取回去（`fetch()` 里只调了 `touch_referenced`），这部分 token **又进模型了**，但没从节省里扣。所以现在展示的是**毛节省**，长期偏乐观。

**方案**：
1. `observations` 加两列：`refetched_tokens INTEGER NOT NULL DEFAULT 0`、`refetch_count INTEGER NOT NULL DEFAULT 0`（schema v9，照 v8 的迁移写法，`ALTER TABLE` + 忽略重复列错误）
2. `Runtime::fetch()` 里，页面命中时把本次交付的 token 数累加到**产生这个 URI 的那条 observation** 上：`UPDATE observations SET refetched_tokens = refetched_tokens + ?, refetch_count = refetch_count + 1 WHERE uri = ? AND id = (SELECT id FROM observations WHERE uri = ? ORDER BY created_at DESC LIMIT 1)`
3. `metrics.rs` 增加 `net_avoided = SUM(avoided_tokens) - SUM(refetched_tokens)`，`dashboard_totals` / `dashboard_models` 一并返回
4. 前端 KPI 第 2 张卡副行显示 `净节省 X · 回取 Y`；成本按**净额**算（`avoided_usd` 用 net），并在 tooltip 里说明
5. 新指标 **回取率** = `refetch_count / 拦截次数`，按 optimizer 分组，进设置页「高级」

**验收**：新增测试 `refetch_is_netted_out_of_savings`：拦截一次省 1000 → 净 1000；`ctx_fetch` 取回 400 → 净 600，毛额仍是 1000。

### D2. 影子模式（dry-run）★直接回答"是不是每次都省了"

**问题**：用户无法验证 CTX 的贡献，只能信数字。

**方案**：`Config.shadow_mode`（Part C5）。在 `runtime.rs` 的 `finish` 之前分叉：照常跑流水线、照常算 `avoided` 并落库（新增 `shadow INTEGER NOT NULL DEFAULT 0` 列标记），但 `IngestResult.delivered` 返回**原文**。仪表盘对影子数据用虚线/斜纹表示，KPI 显示「影子模式：本可节省 X」。

**验收**：`shadow_mode_reports_savings_without_changing_delivery` —— 影子开启时 `result.delivered == 原文` 且 observation 的 `avoided_tokens > 0`。

### D3. 近似去重（SimHash）★省得更多

**问题**：`store.remember_fingerprint(&hash, &normalize_hash(payload), ...)` 是**精确哈希 + 归一化哈希**两级。只要日志里有一个时间戳、耗时、临时路径不同，两次几乎一样的构建输出就会被当成全新内容，`DuplicateGuard` 不生效。

**方案**：
1. `fingerprints` 表加 `simhash INTEGER`（64 位，存 `i64`）+ 4 个分段列 `band0..band3`（每 16 位一段）建索引，实现"近邻查询不用全表扫"
2. 计算：对归一化后的文本做 3-gram shingle（先跑现有 `normalize_hash` 的归一化：数字、hex、路径、时间戳全部替换成占位符），每个 shingle 取 64 位哈希，按位加权求和取符号位 → SimHash
3. 查询：按 4 个 band 任一命中的候选里，算 Hamming 距离 ≤ 3 的最近一条 → 判为近似重复，走 `DuplicateGuard::render`，文案改成「与 ctx://… 近似（差异 N 行）」并附上真实 diff 的前 20 行
4. 阈值可配：`Config.near_duplicate_hamming`（默认 0 = 关闭近似折叠；仅精确/空白归一化合并）

**验收**：两份仅时间戳/耗时不同的 `cargo test` 输出，第二份 `delivered_tokens < 200` 且 `optimizer == "duplicate"`。

### D4. 回取率反馈的自适应预算 ★自动调参

**问题**：`crates/ctx-optimizer/src/budget.rs::cap_for` 是静态公式（15% + 信号行加成 + 策略系数 + clamp）。削太狠会让模型回取，削太松没价值，但系统**从不学习**。

**方案**：
1. 新表 `optimizer_stats(optimizer TEXT PRIMARY KEY, intercepts INTEGER, avoided INTEGER, refetched INTEGER, updated_at INTEGER)`，由 D1 的数据每次 ingest 增量更新
2. `cap_for` 增加一个乘数 `tune ∈ [0.75, 1.4]`：回取率 > 20% → 逐步放宽（`tune += 0.05`，上限 1.4）；回取率 < 5% → 逐步收紧（`tune -= 0.05`，下限 0.75）。每 50 次拦截调整一次，避免抖动
3. `tune` 存 `optimizer_stats`，进程启动时读入；设置页「高级」里展示每个 guard 的当前 `tune` 和回取率，并提供「重置调参」

**验收**：`tune_widens_after_high_refetch` / `tune_tightens_after_clean_run` 两个单测直接喂统计数据断言乘数方向。

### D5. Token 估算分内容类型校准 ★让 $ 更准

**问题**：`crates/ctx-optimizer/src/tokens.rs::estimate_tokens` 全局用 `ascii/3.8 + non_ascii/1.1`，再和 `words × 1.3` 取大。JSON（大量标点）、代码（下划线/驼峰）、中文、base64 的真实 token 密度差异很大，误差直接传到美元数字。

**方案**：把 `estimate_tokens` 改成 `estimate_tokens_for(kind, text)`，按内容类型取不同系数，`kind` 从调用点已有的 `input.kind`（`shell|file|mcp`）+ 轻量嗅探（是否 JSON、CJK 占比、是否 base64 长串）得出：

| 类型 | ascii 除数 | 备注 |
|---|---|---|
| 代码 | 3.3 | 标识符切分多 |
| 日志/shell | 3.9 | |
| JSON | 2.9 | 标点密集 |
| 自然语言（英） | 4.2 | |
| CJK | 非 ascii 除数 0.9 | 中文约 1 字 1.1 token |
| base64/长十六进制 | 3.0 | |

保留 `estimate_tokens` 作为兼容包装（默认走日志系数），避免改所有调用点。同时把校准脚本放 `crates/ctx-optimizer/tests/token_calibration.rs`（用固定语料 + 期望区间断言，不引入 tokenizer 依赖）。

**验收**：现有 `tests/token_bench.rs` 不回归；新增的分类型断言全部落在 ±12% 区间内。

### D6. Cursor 钩子注册补全 ★覆盖率白捡

**问题**：`setup.rs::merge_cursor_hooks` 只注册 6 个事件，而 `hooks.rs` 已经能处理 `afterShellExecution`、`afterMCPExecution`、`preCompact`、`postCompact`。Cursor 里的 shell / MCP 输出现在只能靠 `postToolUse` 兜，拿不到 `exit_code` 等预算信号（`budget::from_parts` 依赖它）。

**方案**：把 Cursor 的注册列表扩到：

```
sessionStart, sessionEnd, preToolUse, postToolUse, postToolUseFailure,
beforeReadFile, beforeSubmitPrompt,
afterShellExecution, afterMCPExecution,
preCompact, subagentStart, afterAgentResponse
```

- `subagentStart` → 读 `subagent_model` 补子代理的模型归属（`hook_model` 已经会读这个字段）
- `afterAgentResponse` → 只用来确认/补齐 model，不做内容处理
- `afterShellExecution`/`afterMCPExecution` 已有 handler，注册即生效
- 注意**去重**：同一份输出可能同时触发 `postToolUse` 和 `afterShellExecution`，靠 content hash 幂等（`remember_fingerprint` 已经能识别），但要新增测试确认不会重复计一次节省

**验收**：`merge_cursor_registers_shell_and_mcp_hooks`；以及 `same_output_from_two_hooks_counts_once`。

### D7. 内容定义分块（CDC）增量文件读

**问题**：`runtime.rs` 里文件未变判断是**整文件 content hash 相等**。改一行就整份重发（`cow`/`diff` 能部分兜住，但依赖符号边界，非代码文件没用）。

**方案**：`file_reads` 加 `chunks TEXT`（存 `[{offset,len,hash}]`）。用 Gear/FastCDC 做内容定义分块（目标块 2KB，min 512B，max 8KB），只交付变化块 + `ctx://` 引用未变块。非代码文本（md/yaml/csv/log）收益最大。

**验收**：2000 行文件改 2 行，第二次读交付 < 300 token 且能通过 `ctx_read` 完整还原。

### D8. 上下文占用感知的动态强度

**问题**：预算策略是全局静态的。对话刚开始时上下文很空，可以保守（保留更多细节）；接近压缩时应该激进。

**方案**：`sessions` 表加 `ctx_used_tokens`、`ctx_window_tokens`（`ctx_window` 从 model id 查一张窗口表，Cursor 的 `model_params` 里还有 `context: "1m"` 可用）。累加本会话交付量估算占用率，在 `budget::cap_for` 里加系数：占用 < 40% → ×1.15；40–70% → ×1.0；> 70% → ×0.8。`preCompact` 事件命中时临时切到激进档。

**验收**：`budget_tightens_as_context_fills`。

### D9. 流水线选择：从"最小 delivered"改成"评分"

**问题**：`crates/ctx-optimizer/src/pipeline.rs::run` 选 `delivered_tokens` **最小**的那个输出（`out.delivered_tokens < b.delivered_tokens`）。这等于"谁删得最狠谁赢"，完全不看是否删掉了关键信号。

**方案**：给 `OptimizeOutput` 加 `signal_kept: u32`（用 `budget::count_signal_lines` 数保留下来的诊断行/错误行），选择时用

```
score = signal_kept * W - delivered_tokens        // W 默认 40，可配
```

`terminal` 语义不变（专用 guard 仍优先）。同时把 `best.filter(|out| out.delivered_tokens + 40 < input.raw_tokens)` 里的魔法数 40 提成常量 `MIN_GAIN_TOKENS` 并注释清楚。

**验收**：构造一个"删得更狠但丢了 `panicked at` 行"的候选，断言流水线选保留了 panic 行的那个。

### D10. 性能护栏

- 钩子端到端 p95 必须 < 25ms（`observe.rs` 已有 `record_hook` 采样）。D3 的 SimHash 和 D7 的 CDC 都是新增 CPU 开销，必须在 `/api/health` 暴露 p50/p95，并在设置页显示
- 单次 payload 超过 2MB 时跳过 SimHash（退回精确哈希），避免长尾卡顿
- 所有新表/新列都要建索引；`observations(model)` 索引已在 v8 建好，注意 v9 的 `refetched_tokens` 不需要索引

---

## Part E：实施顺序与验收

### 阶段划分（每阶段独立可交付、可回滚）

| 阶段 | 内容 | 依赖 |
|---|---|---|
| 1 | Part A 配色 + Part B 布局（纯前端，改 `app.html` + macOS 颜色常量） | 无 |
| 2 | Part C 设置页：Config 字段 + 钩子 gate + `/api/harnesses` `/api/harness` `/api/doctor` + 抽屉 UI | 无 |
| 3 | Claude Desktop 支持 + Cursor 钩子补全（D6）+ **观测幂等唯一索引**（三层钩子双计，见 C3 ⚠️） | 阶段 2 的 harness 目录 |
| 4 | D1 净节省 + D2 影子模式（schema v9） | 阶段 2、3（幂等要先于净额，否则净额也是错的） |
| 5 | D3 SimHash + D4 自适应预算 + D9 评分选择 | 阶段 4 的统计数据 |
| 6 | D5 token 校准 + D7 CDC + D8 占用感知 | 阶段 5 |

### 全局验收清单

- [ ] `cargo test` 全绿；新增功能每项都有测试（上面每节都列了测试名）
- [ ] 配色：任何图形里"绿=省下的、蓝=该有的"，品牌绿不再用于填充
- [ ] 1180 / 1024 / 768 / 375 四个宽度无空白块、无溢出
- [ ] 齿轮 → 抽屉能看到每个工具的**接入状态、能力（自动优化/仅检索）、开关、今日战绩**
- [ ] 关掉某个 harness 后，该工具的钩子确实直通（新增集成测试：`disabled_harness_passes_through`），且 agent 不报错
- [ ] 「仅检索」的工具在 UI 上明确写着"不会自动省 token"
- [ ] KPI 第 4 张卡点击能直达设置的工具分区
- [ ] 净节省与毛节省同时可见，成本按净额计算，估算带「估」角标
- [ ] 同一事件被两层 hooks.json 触发两次时，只记一条节省（测试 `same_event_from_two_hook_levels_counts_once`）；设置页显示注册层级并对重复注册给出提示
- [ ] 钩子 p95 < 25ms，`/api/health` 可查
- [ ] 影子模式下 agent 收到的内容与未装 CTX 完全一致

### 不要做的事

- 不要引入前端框架 / 打包器：仪表盘是 `include_str!` 的单文件，保持这样
- 不要引入真 tokenizer 依赖（体积和构建时间不值当），用校准系数
- 不要在钩子路径上做网络请求（价格刷新只在仪表盘/后台做）
- 不要把 `enabled` 的语义改成 per-harness——全局开关保留，per-harness 是叠加在它之上的
- 钩子任何分支都必须 fail-open，宁可不优化也不能让 agent 卡住
