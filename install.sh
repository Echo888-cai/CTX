#!/usr/bin/env bash
# One-shot install for CTX. Local only. No cloud.
set -euo pipefail

REPO="${CTX_REPO:-https://github.com/Echo888-cai/CTX.git}"
SRC="${CTX_SRC:-$HOME/.ctx/src}"

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

main() {
  ensure_rust
  ensure_path_hint
  resolve_src
  say "building ctx"
  cargo install --path "$SRC/apps/cli" --locked --force
  say "creating ~/.ctx and wiring Claude / Cursor"
  ctx init
  cat <<'EOF'

CTX is on this machine.

  ctx app              open today's avoided-token dashboard
  ctx app --install-service   start it at login (macOS / Linux)
  ctx status           same numbers in the terminal
  ctx doctor           wiring check

The big number is tokens that never entered the model.
Raw context stayed on disk. Nothing was summarized away.
EOF
}

main "$@"
