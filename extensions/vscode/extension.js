const vscode = require("vscode");
const { spawn, spawnSync } = require("child_process");

function ctxBin() {
  const cfg = vscode.workspace.getConfiguration("ctx");
  return cfg.get("bin") || "ctx";
}

function runCtx(args, cwd) {
  const r = spawnSync(ctxBin(), args, {
    encoding: "utf8",
    cwd: cwd || vscode.workspace.workspaceFolders?.[0]?.uri.fsPath,
    timeout: 15_000,
  });
  if (r.error) throw r.error;
  return { stdout: r.stdout || "", stderr: r.stderr || "", status: r.status };
}

function esc(s) {
  return String(s ?? "").replace(/[&<>]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;" }[c]));
}

function inspectHtml(payload, fallback) {
  let pages = [];
  try {
    const data = typeof payload === "string" ? JSON.parse(payload) : payload;
    pages = Array.isArray(data.pages) ? data.pages : [];
  } catch {
    return `<html><body style="font:13px/1.45 ui-monospace,monospace;white-space:pre-wrap;padding:16px;background:#111;color:#e8e8e8">${esc(fallback)}</body></html>`;
  }
  const groups = { HOT: [], WARM: [], COLD: [] };
  for (const p of pages) {
    const layer = String(p.layer || "COLD").toUpperCase();
    (groups[layer] || groups.COLD).push(p);
  }
  const section = (name, color) => {
    const rows = groups[name]
      .map(
        (p) =>
          `<tr><td>${esc(p.label || p.frame || "")}</td><td><code>${esc(p.uri)}</code></td><td>${esc(p.tokens)}</td><td>${esc(p.harness || "")}</td></tr>`
      )
      .join("");
    return `<h2 style="color:${color};margin:20px 0 8px">${name} <small style="opacity:.6">${groups[name].length}</small></h2>
      <table><thead><tr><th>Page</th><th>URI</th><th>Tokens</th><th>Harness</th></tr></thead><tbody>${rows || `<tr><td colspan="4" style="opacity:.5">empty</td></tr>`}</tbody></table>`;
  };
  return `<!DOCTYPE html><html><head><style>
    body{font:13px/1.45 ui-sans-serif,system-ui;background:#111;color:#e8e8e8;padding:20px;margin:0}
    table{width:100%;border-collapse:collapse}
    th,td{text-align:left;padding:6px 8px;border-bottom:1px solid #2a2a2a;vertical-align:top}
    code{font:12px ui-monospace,monospace;color:#9cdcfe}
    h1{font-size:16px;font-weight:600;margin:0 0 4px}
  </style></head><body>
    <h1>CTX working set</h1>
    <p style="opacity:.7;margin:0 0 16px">Pages the model can page in. Raw dumps stay on disk.</p>
    ${section("HOT", "#f97316")}
    ${section("WARM", "#eab308")}
    ${section("COLD", "#64748b")}
  </body></html>`;
}

function activate(context) {
  const bar = vscode.window.createStatusBarItem(vscode.StatusBarAlignment.Right, 80);
  bar.command = "ctx.status";
  bar.text = "CTX";
  bar.tooltip = "CTX avoided tokens today";
  bar.show();
  context.subscriptions.push(bar);

  const refresh = () => {
    try {
      const r = runCtx(["status", "--json"]);
      const data = JSON.parse(r.stdout || "{}");
      const n = data.today?.avoided ?? data.avoided ?? data.totals?.avoided;
      if (typeof n === "number") {
        const label = n >= 1000 ? `${(n / 1000).toFixed(1)}K` : String(n);
        bar.text = `CTX ${label}`;
      }
    } catch {
      bar.text = "CTX";
    }
  };
  refresh();
  const timer = setInterval(refresh, 60_000);
  context.subscriptions.push({ dispose: () => clearInterval(timer) });

  const show = (title, body) => {
    const doc = vscode.window.createOutputChannel(title);
    doc.clear();
    doc.append(body);
    doc.show(true);
  };

  context.subscriptions.push(
    vscode.commands.registerCommand("ctx.install", async () => {
      const r = runCtx(["setup", "all"]);
      vscode.window.showInformationMessage(r.status === 0 ? "CTX hooks installed" : r.stderr || r.stdout);
    }),
    vscode.commands.registerCommand("ctx.uninstall", async () => {
      const r = runCtx(["uninstall"]);
      vscode.window.showInformationMessage(r.status === 0 ? "CTX hooks removed" : r.stderr || r.stdout);
    }),
    vscode.commands.registerCommand("ctx.dashboard", async () => {
      spawn(ctxBin(), ["app"], { detached: true, stdio: "ignore" }).unref();
      const panel = vscode.window.createWebviewPanel(
        "ctxDashboard",
        "CTX dashboard",
        vscode.ViewColumn.Active,
        { enableScripts: true }
      );
      panel.webview.html = `<!DOCTYPE html><html><body style="margin:0;background:#111">
        <iframe src="http://127.0.0.1:8741" style="border:0;width:100%;height:100vh"></iframe>
      </body></html>`;
      vscode.env.openExternal(vscode.Uri.parse("http://127.0.0.1:8741"));
    }),
    vscode.commands.registerCommand("ctx.status", async () => {
      const r = runCtx(["status"]);
      show("CTX status", r.stdout || r.stderr);
    }),
    vscode.commands.registerCommand("ctx.inspect", async () => {
      const r = runCtx(["inspect", "--json"]);
      const panel = vscode.window.createWebviewPanel("ctxInspect", "CTX working set", vscode.ViewColumn.Beside, {});
      panel.webview.html = inspectHtml(r.stdout, r.stdout || r.stderr);
    }),
    vscode.commands.registerCommand("ctx.readFile", async (uri) => {
      const file = uri?.fsPath || vscode.window.activeTextEditor?.document.uri.fsPath;
      if (!file) {
        vscode.window.showWarningMessage("No file selected");
        return;
      }
      const r = runCtx(["read", file]);
      const doc = await vscode.workspace.openTextDocument({ content: r.stdout || r.stderr, language: "markdown" });
      vscode.window.showTextDocument(doc, { preview: true });
    })
  );
}

function deactivate() {}

module.exports = { activate, deactivate, ctxBin, inspectHtml };
