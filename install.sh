#!/usr/bin/env bash
# One-shot install for CTX. Local only. No cloud.
set -euo pipefail

REPO="${CTX_REPO:-https://github.com/Echo888-cai/CTX.git}"
SRC="${CTX_SRC:-$HOME/.ctx/src}"
GH_DOWNLOAD="${REPO%.git}"

say() { printf '==> %s\n' "$*"; }
die() { printf 'ctx install: %s\n' "$*" >&2; exit 1; }

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || die "need $1"
}

ensure_rust() {
  if command -v cargo >/dev/null 2>&1 && command -v rustc >/dev/null 2>&1; then
    return
  fi
  say "installing Rust (rustup)"
  need_cmd curl
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable
  # shellcheck disable=SC1091
  . "$HOME/.cargo/env"
  command -v cargo >/dev/null 2>&1 || die "cargo still missing after rustup"
}

ensure_path_hint() {
  case ":$PATH:" in
    *":$HOME/.cargo/bin:"*) ;;
    *)
      printf '\nAdd this to your shell profile:\n  export PATH="$HOME/.cargo/bin:$PATH"\n\n'
      export PATH="$HOME/.cargo/bin:$PATH"
      ;;
  esac
}

resolve_src() {
  local here
  here="$(cd "$(dirname "$0")" && pwd)"
  if [ -f "$here/apps/cli/Cargo.toml" ]; then
    SRC="$here"
    return
  fi
  need_cmd git
  if [ -d "$SRC/.git" ]; then
    say "updating $SRC"
    git -C "$SRC" pull --ff-only
  else
    say "cloning $REPO"
    mkdir -p "$(dirname "$SRC")"
    git clone --depth 1 "$REPO" "$SRC"
  fi
}

latest_tag() {
  local url
  url="$(curl -fsSLI -o /dev/null -w '%{url_effective}' "$GH_DOWNLOAD/releases/latest")" || return 1
  url="${url%%$'\r'}"
  url="${url%/}"
  printf '%s\n' "${url##*/}"
}

install_cli_from_tar() {
  local archive="$1" tmp
  tmp="$(mktemp -d)"
  tar -C "$tmp" -xzf "$archive"
  mkdir -p "$HOME/.cargo/bin"
  if [ -f "$tmp/ctx" ]; then
    install -m 755 "$tmp/ctx" "$HOME/.cargo/bin/ctx"
    rm -rf "$tmp"
    return 0
  fi
  local found
  found="$(find "$tmp" -maxdepth 2 -type f \( -name ctx -o -name 'ctx-*' \) | head -1)"
  if [ -n "$found" ]; then
    install -m 755 "$found" "$HOME/.cargo/bin/ctx"
    rm -rf "$tmp"
    return 0
  fi
  rm -rf "$tmp"
  return 1
}

fetch_prebuilt() {
  local file="$1" tag="$2" url tmp
  url="$GH_DOWNLOAD/releases/download/${tag}/${file}"
  tmp="$(mktemp -d)"
  if curl -fsSL "$url" -o "$tmp/ctx.tar.gz"; then
    if install_cli_from_tar "$tmp/ctx.tar.gz"; then
      rm -rf "$tmp"
      say "installed prebuilt ctx ($file)"
      return 0
    fi
  fi
  rm -rf "$tmp"
  return 1
}

try_prebuilt() {
  local os arch tag ver
  os="$(uname -s)"
  arch="$(uname -m)"
  command -v curl >/dev/null 2>&1 || return 1
  tag="$(latest_tag)" || return 1
  ver="${tag#v}"
  CTX_RELEASE_TAG="$tag"
  CTX_RELEASE_VER="$ver"
  case "$os-$arch" in
    Darwin-arm64) fetch_prebuilt "CTX-Apple-Arm-cli-v${ver}.tar.gz" "$tag" ;;
    Darwin-x86_64) fetch_prebuilt "CTX-Apple-Intel-cli-v${ver}.tar.gz" "$tag" ;;
    Linux-x86_64)
      fetch_prebuilt "CTX-Linux-x64-v${ver}.tar.gz" "$tag" \
        || fetch_prebuilt "CTX-Linux-x64-musl-v${ver}.tar.gz" "$tag"
      ;;
    Linux-aarch64) fetch_prebuilt "CTX-Linux-Arm-v${ver}.tar.gz" "$tag" ;;
    *) return 1 ;;
  esac
}

install_mac_app() {
  [ "$(uname -s)" = Darwin ] || return 0
  command -v curl >/dev/null 2>&1 || return 0
  command -v ditto >/dev/null 2>&1 || return 0
  local name url tmp dest tag ver
  tag="${CTX_RELEASE_TAG:-$(latest_tag || true)}"
  [ -n "$tag" ] || return 0
  ver="${tag#v}"
  case "$(uname -m)" in
    arm64) name="CTX-Apple-Arm-v${ver}" ;;
    x86_64) name="CTX-Apple-Intel-v${ver}" ;;
    *) return 0 ;;
  esac
  tmp="$(mktemp -d)"
  url="$GH_DOWNLOAD/releases/download/${tag}/${name}.zip"
  if ! curl -fsSL "$url" -o "$tmp/ctx.zip"; then
    rm -rf "$tmp"
    return 0
  fi
  mkdir -p "$tmp/out" "$HOME/Applications"
  ditto -x -k "$tmp/ctx.zip" "$tmp/out" 2>/dev/null || true
  if [ ! -d "$tmp/out/CTX.app" ]; then
    rm -rf "$tmp"
    return 0
  fi
  dest="$HOME/Applications/CTX.app"
  rm -rf "$dest"
  cp -R "$tmp/out/CTX.app" "$dest"
  xattr -dr com.apple.quarantine "$dest" 2>/dev/null || true
  if [ -w /Applications ]; then
    rm -rf /Applications/CTX.app
    cp -R "$tmp/out/CTX.app" /Applications/CTX.app
    xattr -dr com.apple.quarantine /Applications/CTX.app 2>/dev/null || true
    dest="/Applications/CTX.app"
  fi
  rm -rf "$tmp"
  say "installed CTX.app → $dest"
  open "$dest" >/dev/null 2>&1 || true
}

print_done() {
  cat <<'EOF'

CTX is on this machine.

  Mac: open CTX-Apple-Arm-v*.dmg and drag CTX.app into Applications
  ctx app                 open today's avoided-token dashboard
  ctx app --install-app   rebuild the Dock app from this tree
  ctx app --install-service   start the dashboard at login (macOS / Linux)
  ctx status              same numbers in the terminal
  ctx doctor              wiring check

The big number is tokens that never entered the model.
Raw context stayed on disk. Nothing was summarized away.
EOF
}

main() {
  CTX_RELEASE_TAG=""
  CTX_RELEASE_VER=""
  if try_prebuilt; then
    ensure_path_hint
    say "creating ~/.ctx and wiring Claude / Cursor"
    ctx init
    install_mac_app
    print_done
    return
  fi
  ensure_rust
  ensure_path_hint
  resolve_src
  say "building ctx"
  cargo install --path "$SRC/apps/cli" --locked --force
  say "creating ~/.ctx and wiring Claude / Cursor"
  ctx init
  install_mac_app
  print_done
}

main "$@"
