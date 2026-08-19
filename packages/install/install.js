#!/usr/bin/env node
"use strict";

const { spawnSync } = require("child_process");
const os = require("os");
const path = require("path");
const fs = require("fs");
const https = require("https");
const { pipeline } = require("stream");
const { promisify } = require("util");
const pipe = promisify(pipeline);

const VERSION = process.env.CTX_VERSION || "0.2.0";
const REPO = "Echo888-cai/CTX";

function targets() {
  const plat = os.platform();
  const arch = os.arch();
  if (plat === "darwin" && arch === "arm64") return ["aarch64-apple-darwin"];
  if (plat === "darwin") return ["x86_64-apple-darwin"];
  if (plat === "linux" && arch === "arm64") return ["aarch64-unknown-linux-gnu"];
  if (plat === "linux") return ["x86_64-unknown-linux-gnu", "x86_64-unknown-linux-musl"];
  if (plat === "win32") return ["x86_64-pc-windows-msvc"];
  throw new Error(`unsupported ${plat}-${arch}`);
}

function destDir() {
  if (os.platform() === "win32") {
    const dir = path.join(os.homedir(), "AppData", "Local", "ctx", "bin");
    fs.mkdirSync(dir, { recursive: true });
    return dir;
  }
  const dir = path.join(os.homedir(), ".local", "bin");
  fs.mkdirSync(dir, { recursive: true });
  return dir;
}

function download(url, file) {
  return new Promise((resolve, reject) => {
    https
      .get(url, (res) => {
        if (res.statusCode >= 300 && res.statusCode < 400 && res.headers.location) {
          res.resume();
          return download(res.headers.location, file).then(resolve, reject);
        }
        if (res.statusCode !== 200) {
          reject(new Error(`GET ${url} -> ${res.statusCode}`));
          res.resume();
          return;
        }
        pipe(res, fs.createWriteStream(file)).then(resolve, reject);
      })
      .on("error", reject);
  });
}

function cargoFallback() {
  console.error("prebuilt missing; falling back to cargo install");
  const r = spawnSync(
    "cargo",
    ["install", "--git", `https://github.com/${REPO}`, "--locked", "--force", "ctx-cli"],
    { stdio: "inherit" }
  );
  process.exit(r.status || 1);
}

async function installTarget(t) {
  const win = t.includes("windows");
  const url = win
    ? `https://github.com/${REPO}/releases/download/v${VERSION}/ctx-${t}.exe`
    : `https://github.com/${REPO}/releases/download/v${VERSION}/ctx-${t}.tar.gz`;
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-"));
  const archive = path.join(tmp, win ? "ctx.exe" : "ctx.tar.gz");
  await download(url, archive);
  const dest = path.join(destDir(), win ? "ctx.exe" : "ctx");
  if (win) {
    fs.copyFileSync(archive, dest);
  } else {
    const r = spawnSync("tar", ["-xzf", archive, "-C", tmp], { stdio: "inherit" });
    if (r.status !== 0) throw new Error("tar failed");
    const names = fs.readdirSync(tmp);
    const binName = names.find((n) => n.startsWith("ctx") && !n.endsWith(".tar.gz")) || "ctx";
    fs.copyFileSync(path.join(tmp, binName), dest);
    fs.chmodSync(dest, 0o755);
  }
  return dest;
}

async function main() {
  const list = targets();
  let dest;
  for (const t of list) {
    try {
      dest = await installTarget(t);
      break;
    } catch (err) {
      console.error(`${t}: ${err.message}`);
    }
  }
  if (!dest) cargoFallback();
  console.log(`installed ${dest}`);
  spawnSync(dest, ["init"], { stdio: "inherit" });
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
