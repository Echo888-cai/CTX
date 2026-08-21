#!/bin/bash
# Copy CTX.app into /Applications and clear the browser quarantine flag.
set -euo pipefail
cd "$(dirname "$0")"
if [ ! -d CTX.app ]; then
  printf 'ctx: CTX.app not found next to this installer.\n' >&2
  exit 1
fi
xattr -dr com.apple.quarantine CTX.app 2>/dev/null || true
DEST="/Applications/CTX.app"
rm -rf "$DEST"
cp -R CTX.app "$DEST"
xattr -dr com.apple.quarantine "$DEST" 2>/dev/null || true
open "$DEST"
