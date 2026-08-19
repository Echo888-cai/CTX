const vscode = require("vscode");
const { spawn, spawnSync } = require("child_process");
const path = require("path");

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
      vscode.env.openExternal(vscode.Uri.parse("http://127.0.0.1:8741"));
    }),
    vscode.commands.registerCommand("ctx.status", async () => {
      const r = runCtx(["status"]);
      show("CTX status", r.stdout || r.stderr);
    }),
    vscode.commands.registerCommand("ctx.inspect", async () => {
      const r = runCtx(["inspect"]);
      const panel = vscode.window.createWebviewPanel("ctxInspect", "CTX working set", vscode.ViewColumn.Beside, {});
      const body = (r.stdout || r.stderr || "").replace(/[&<>]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;" }[c]));
      panel.webview.html = `<html><body style="font:13px/1.45 ui-monospace,monospace;white-space:pre-wrap;padding:16px">${body}</body></html>`;
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

module.exports = { activate, deactivate, ctxBin };
