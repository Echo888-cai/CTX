# Changelog

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
