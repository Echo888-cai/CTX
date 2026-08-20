#!/usr/bin/env bash
# Build the native macOS menu bar app (not Electron/Tauri).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")" && pwd)"
DIST="${CTX_APP_DIST:-$ROOT/dist/CTX.app}"
INSTALL="${HOME}/Applications/CTX.app"

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
  -o "$BIN_DIR/CTX" \
  "$ROOT"/Sources/*.swift

rm -rf "$DIST"
mkdir -p "$DIST/Contents/MacOS" "$DIST/Contents/Resources"
cp "$ROOT/Info.plist" "$DIST/Contents/Info.plist"
cp "$BIN_DIR/CTX" "$DIST/Contents/MacOS/CTX"
cp "$ROOT/../cli/src/assets/ctx-wordmark.png" "$DIST/Contents/Resources/ctx-wordmark.png"
chmod +x "$DIST/Contents/MacOS/CTX"

say "built $DIST"

if [ "${1:-}" = "--install" ]; then
  mkdir -p "$(dirname "$INSTALL")"
  rm -rf "$INSTALL"
  cp -R "$DIST" "$INSTALL"
  say "installed $INSTALL"
  open "$INSTALL"
  say "look for ↓% in the menu bar"
fi
