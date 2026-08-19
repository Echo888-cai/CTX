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

function target() {
  const plat = os.platform();
  const arch = os.arch();
  if (plat === "darwin" && arch === "arm64") return "aarch64-apple-darwin";
  if (plat === "darwin") return "x86_64-apple-darwin";
  if (plat === "linux" && arch === "arm64") return "aarch64-unknown-linux-gnu";
  if (plat === "linux") return "x86_64-unknown-linux-gnu";
  throw new Error(`unsupported ${plat}-${arch}`);
}

function destDir() {
  const home = os.homedir();
  const dir = path.join(home, ".local", "bin");
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

async function main() {
  const t = target();
  const url = `https://github.com/${REPO}/releases/download/v${VERSION}/ctx-${t}.tar.gz`;
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-"));
  const tar = path.join(tmp, "ctx.tar.gz");
  try {
    await download(url, tar);
  } catch (err) {
    console.error("prebuilt missing; falling back to cargo install");
    const r = spawnSync("cargo", ["install", "--git", `https://github.com/${REPO}`, "--locked", "--force", "ctx-cli"], {
      stdio: "inherit",
    });
    process.exit(r.status || 1);
  }
  const r = spawnSync("tar", ["-xzf", tar, "-C", tmp], { stdio: "inherit" });
  if (r.status !== 0) process.exit(r.status || 1);
  const names = fs.readdirSync(tmp);
  const binName = names.find((n) => n.startsWith("ctx") && !n.endsWith(".tar.gz")) || "ctx";
  const dest = path.join(destDir(), "ctx");
  fs.copyFileSync(path.join(tmp, binName), dest);
  fs.chmodSync(dest, 0o755);
  console.log(`installed ${dest}`);
  spawnSync(dest, ["init"], { stdio: "inherit" });
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
