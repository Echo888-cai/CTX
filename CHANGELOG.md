# Changelog

## 0.1.4

- Dashboard filters: source → model → range; Claude/Codex model lists come from ledger only; model labels Title-Case.
- DeepSeek V4 pricing: `deepseek-v4-pro` in catalog; flash/pro cache_read/output/write extras; DeepSeek tier rates (not generic).
- Claude cache_write parsing avoids double-counting 1h creation; blank-model Claude observations attribute via ledger sessions.
- Cache heat: hit % + Token 消耗 + 命中 + 账单; CC Switch–aligned token/cost math.

## 0.1.3

- Dashboard ledger syncs live (~1s): Claude / Codex from provider usage, Cursor from context-window prefix inference when local `cache_read` is missing.
- Cache hit rate matches CC Switch: `cache_read / (fresh + cache_write + cache_read)`; date windows use `until`; local timezone for「当天」.
- Pricing refresh can overlay models.dev cache/output tiers; empty ledger states explain measured vs inferred.

## 0.1.2

- Opening CTX.app wires detected IDEs (Cursor, Claude Code, ChatGPT). Quitting pauses saving. Deleting the app restores those IDE configs.

## 0.1.1

- Dashboard: date range waits for Confirm; model list uses `Other`; enable toast is a flip badge, not a green wash.
- macOS icon is a full-bleed square so Dock / Launchpad do not nest a rounded square inside the system squircle.
- DMG includes `Install CTX.command`, which copies the app and clears the download quarantine flag.

## 0.1

- Public installer release: `CTX-Apple-Arm-v0.1.dmg`, `CTX-Apple-Intel-v0.1.dmg`, `CTX-Windows-x64-v0.1.exe`, `CTX-Linux-x64-v0.1.tar.gz`.
- macOS archives unpack to `ctx` / `CTX.app`. The rustc triple (`ctx-aarch64-apple-darwin`) is not the download name.

## 0.2.2

- GitHub Release installers use readable names: `CTX-Apple-Arm-v0.2.2.dmg`, `CTX-Apple-Intel-v0.2.2.dmg`, `CTX-Windows-x64-v0.2.2.exe`, `CTX-Linux-x64-v0.2.2.tar.gz`.
- macOS archives unpack to `ctx` / `CTX.app`. The rustc triple (`ctx-aarch64-apple-darwin`) is no longer the download name.
- Apple Silicon and Intel Mac apps both build on `macos-latest`, so the Arm DMG actually publishes.

## 0.2.1

- Standalone macOS app: drag-install from `CTX-macOS-arm64.dmg` (Intel: `CTX-macOS-x86_64.dmg`), or `ctx app --install-app`.
- Dashboard cache-hit rate, with a seamless flowing-text shine on the percentage.
- Model merge in telemetry / ledger pricing.
- Release assets: Mac drag-install DMG, CLI tarballs (macOS, Linux, Windows).
