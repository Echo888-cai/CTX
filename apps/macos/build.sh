#!/usr/bin/env bash
# Build the native macOS menu bar app (not Electron/Tauri).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")" && pwd)"
DIST="${CTX_APP_DIST:-$ROOT/dist/CTX.app}"
INSTALL="${HOME}/Applications/CTX.app"
ASSETS="$ROOT/../cli/src/assets"

say() { printf '==> %s\n' "$*"; }
die() { printf 'ctx bar: %s\n' "$*" >&2; exit 1; }

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || die "need $1"
}

need_cmd swiftc
need_cmd xcrun

SDK="$(xcrun --sdk macosx --show-sdk-path)"
ARCH="$(uname -m)"
TARGET="${ARCH}-apple-macosx13.0"
BIN_DIR="$(mktemp -d)"
trap 'rm -rf "$BIN_DIR"' EXIT

say "compiling CTX menu bar ($TARGET)"
# shellcheck disable=SC2086
swiftc -parse-as-library \
  -O \
  -sdk "$SDK" \
  -target "$TARGET" \
  -framework SwiftUI \
  -framework AppKit \
  -o "$BIN_DIR/CTX" \
  "$ROOT"/Sources/*.swift

rm -rf "$DIST"
mkdir -p "$DIST/Contents/MacOS" "$DIST/Contents/Resources"
cp "$ROOT/Info.plist" "$DIST/Contents/Info.plist"
cp "$BIN_DIR/CTX" "$DIST/Contents/MacOS/CTX"
for asset in ctx-wordmark.png ctx-menubar.png ctx-mark.png; do
  if [ -f "$ASSETS/$asset" ]; then
    cp "$ASSETS/$asset" "$DIST/Contents/Resources/$asset"
  fi
done
chmod +x "$DIST/Contents/MacOS/CTX"

if command -v codesign >/dev/null 2>&1; then
  codesign --force --deep --sign - "$DIST" >/dev/null 2>&1 || true
fi

say "built $DIST"

if [ "${1:-}" = "--install" ]; then
  pkill -f 'Applications/CTX.app/Contents/MacOS/CTX' 2>/dev/null || true
  launchctl bootout "gui/$(id -u)/ai.ctx.bar" 2>/dev/null || true
  sleep 0.3

  mkdir -p "$(dirname "$INSTALL")"
  rm -rf "$INSTALL"
  cp -R "$DIST" "$INSTALL"
  xattr -dr com.apple.quarantine "$INSTALL" 2>/dev/null || true
  say "installed $INSTALL"

  PLIST="${HOME}/Library/LaunchAgents/ai.ctx.bar.plist"
  cat > "$PLIST" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>ai.ctx.bar</string>
  <key>ProgramArguments</key>
  <array>
    <string>${INSTALL}/Contents/MacOS/CTX</string>
  </array>
  <key>RunAtLoad</key>
  <true/>
  <key>KeepAlive</key>
  <true/>
</dict>
</plist>
PLIST
  launchctl bootstrap "gui/$(id -u)" "$PLIST" 2>/dev/null \
    || launchctl load -w "$PLIST" 2>/dev/null \
    || open "$INSTALL"
  launchctl kickstart -k "gui/$(id -u)/ai.ctx.bar" 2>/dev/null || open "$INSTALL"
  say "look for the CTX mark next to the clock"
fi
