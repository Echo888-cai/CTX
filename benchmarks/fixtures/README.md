# ShellGuard fixtures

These are captured or reconstructed from real runner output. Optimizer tests load them and assert two things:

1. Failures / rustc spans survive.
2. Passing-test noise does not.

| File | What it proves |
|---|---|
| `cargo-workspace-pass.txt` | Multi-crate `cargo test` → crate summary, drop `... ok` |
| `cargo-test-fail.txt` | Keep panic/assert; drop passing `--nocapture` stdout |
| `cargo-compile-error.txt` | Keep `error[E0308]` spans; drop `Checking` |
| `pytest-fail.txt` | Keep assertion details, not just FAILED lines |
| `jest-fail.txt` | Keep Expected/Received |
| `nextest-fail.txt` | Keep FAIL stdout, drop PASS lines |
