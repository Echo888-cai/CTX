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
need_cmd hdiutil

detach_dmg() {
  local mount="$1" i
  for i in 1 2 3 4 5 6 7 8; do
    if hdiutil detach "$mount" -force >/dev/null 2>&1; then
      return 0
    fi
    sleep 1
  done
  diskutil unmount force "$mount" >/dev/null 2>&1 || true
  hdiutil detach "$mount" -force >/dev/null 2>&1 || return 1
}

package_dmg() {
  local vol="Install CTX"
  local final rw stage mount
  final="$(cd "$(dirname "$DIST")" && pwd)/CTX.dmg"
  rw="${final%.dmg}.rw.dmg"
  stage="$(mktemp -d)"

  cp -R "$DIST" "$stage/CTX.app"
  ln -s /Applications "$stage/Applications"

  rm -f "$rw" "$final"

  # GitHub runners keep Finder's hold on the RW image; skip layout and write UDZO directly.
  if [ -n "${CI:-}" ]; then
    hdiutil create -volname "$vol" -srcfolder "$stage" -ov -format UDZO -imagekey zlib-level=9 "$final" >/dev/null
    rm -rf "$stage"
    say "packed $final"
    return
  fi

  hdiutil create -volname "$vol" -srcfolder "$stage" -ov -format UDRW "$rw" >/dev/null
  rm -rf "$stage"

  mount="$(hdiutil attach -readwrite -noverify -noautoopen "$rw" | grep -oE '/Volumes/.+$' | tail -1)"
  osascript >/dev/null 2>&1 <<EOF || true
tell application "Finder"
  tell disk "$vol"
    open
    set current view of container window to icon view
    set toolbar visible of container window to false
    set statusbar visible of container window to false
    set bounds of container window to {280, 140, 900, 520}
    set arrangement of icon view options of container window to not arranged
    set icon size of icon view options of container window to 104
    set position of item "CTX.app" of container window to {150, 180}
    set position of item "Applications" of container window to {430, 180}
    close
    open
    delay 1
    close
  end tell
end tell
EOF
  sync
  sleep 1
  detach_dmg "$mount"
  hdiutil convert "$rw" -format UDZO -imagekey zlib-level=9 -o "$final" >/dev/null
  rm -f "$rw"
  say "packed $final"
}

SDK="$(xcrun --sdk macosx --show-sdk-path)"
ARCH="${CTX_APP_ARCH:-$(uname -m)}"
case "$ARCH" in
  aarch64|arm64) ARCH=arm64 ;;
  x86_64|amd64) ARCH=x86_64 ;;
esac
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
VER="${CTX_APP_VERSION:-}"
if [ -z "$VER" ] && [ -f "$ROOT/../../Cargo.toml" ]; then
  VER="$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$ROOT/../../Cargo.toml" | head -1)"
fi
if [ -n "$VER" ] && [ -x /usr/libexec/PlistBuddy ]; then
  /usr/libexec/PlistBuddy -c "Set :CFBundleShortVersionString $VER" "$DIST/Contents/Info.plist"
  /usr/libexec/PlistBuddy -c "Set :CFBundleVersion $VER" "$DIST/Contents/Info.plist"
fi
cp "$BIN_DIR/CTX" "$DIST/Contents/MacOS/CTX"
for asset in ctx-wordmark.png ctx-menubar.png ctx-mark.png ctx-appicon.png; do
  if [ -f "$ASSETS/$asset" ]; then
    cp "$ASSETS/$asset" "$DIST/Contents/Resources/$asset"
  fi
done

ICON_SRC="$ROOT/Resources/AppIcon.png"
if [ ! -f "$ICON_SRC" ]; then
  ICON_SRC="$ASSETS/ctx-appicon.png"
fi
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
package_dmg

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
