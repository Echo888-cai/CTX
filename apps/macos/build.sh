#!/usr/bin/env bash
# Build the standalone macOS app (Dock + main window + menu bar tray).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")" && pwd)"
DIST="${CTX_APP_DIST:-$ROOT/dist/CTX.app}"
INSTALL="${HOME}/Applications/CTX.app"
ASSETS="$ROOT/../cli/src/assets"

say() { printf '==> %s\n' "$*"; }
die() { printf 'ctx app: %s\n' "$*" >&2; exit 1; }

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

say "compiling CTX.app ($TARGET)"
# shellcheck disable=SC2086
swiftc -parse-as-library \
  -O \
  -sdk "$SDK" \
  -target "$TARGET" \
  -framework SwiftUI \
  -framework AppKit \
  -framework WebKit \
  -o "$BIN_DIR/CTX" \
  "$ROOT"/Sources/*.swift

rm -rf "$DIST"
mkdir -p "$DIST/Contents/MacOS" "$DIST/Contents/Resources"
cp "$ROOT/Info.plist" "$DIST/Contents/Info.plist"
cp "$BIN_DIR/CTX" "$DIST/Contents/MacOS/CTX"
for asset in ctx-wordmark.png ctx-menubar.png ctx-mark.png ctx-appicon.png; do
  if [ -f "$ASSETS/$asset" ]; then
    cp "$ASSETS/$asset" "$DIST/Contents/Resources/$asset"
  fi
done

ICON_SRC="$ASSETS/ctx-appicon.png"
if [ ! -f "$ICON_SRC" ]; then
  ICON_SRC="$ASSETS/ctx-mark.png"
fi
if [ -f "$ICON_SRC" ] && command -v sips >/dev/null && command -v iconutil >/dev/null; then
  ICONSET="$BIN_DIR/AppIcon.iconset"
  mkdir -p "$ICONSET"
  for size in 16 32 64 128 256 512; do
    sips -z "$size" "$size" "$ICON_SRC" --out "$ICONSET/icon_${size}x${size}.png" >/dev/null
    double=$((size * 2))
    if [ "$double" -le 1024 ]; then
      sips -z "$double" "$double" "$ICON_SRC" --out "$ICONSET/icon_${size}x${size}@2x.png" >/dev/null
    fi
  done
  iconutil -c icns -o "$DIST/Contents/Resources/AppIcon.icns" "$ICONSET" >/dev/null 2>&1 || true
fi

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
  if [ -w /Applications ]; then
    rm -rf /Applications/CTX.app
    cp -R "$DIST" /Applications/CTX.app
    xattr -dr com.apple.quarantine /Applications/CTX.app 2>/dev/null || true
    INSTALL="/Applications/CTX.app"
  fi
  say "installed $INSTALL"

  defaults write com.apple.controlcenter "NSStatusItem Visible ai.ctx.bar" -bool true
  defaults write com.apple.controlcenter "NSStatusItem VisibleCC ai.ctx.bar" -bool true
  defaults write com.apple.controlcenter "NSStatusItem Preferred Position ai.ctx.bar" -float 700
  open "$INSTALL"
  say "CTX.app is a regular Mac app — Dock + main window, like CC Switch"
fi
