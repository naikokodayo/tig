# Split main history refresh receipt

Base: `b6376f58bea6c2837d0c12336edb0c52ef6f8459` (`origin/main`). The focused main pane now uses the existing cancellable Git history task in a split. Loading, cancellation, and Git errors retain both panes; completion renders at the main pane's width. Child-focused refresh, periodic refresh, and initial loading remain synchronous.

| File | SHA-256 |
| --- | --- |
| `rust/main.rs` | `49a8b47cd179a6d3858a8366e3769e2e77a65505a4bcad2da0f08c60727f27a8` |
| `rust/tests/main-refresh-pty.py` | `c3ce10fbcb5062143d82458bf7a059924a2b1f3f6e6540793b52426827b47705` |
| `src/tig` (C binary) | `54db05e55837e0e9ea34d91b08957faca549285e437683cd31fc504f222b6dd1` |
| `target/release/tig` (Rust binary) | `738c8979a8c1a68714e50315f711443b4295c9bf9fda4ec05c475be282c0470e` |

Checks: `cargo fmt --check`, `cargo test` (125 tests), `cargo clippy --all-targets -- -D warnings`, and `cargo build --release` passed. Original `test/main/refresh-test` passed 8/8 assertions in C and Rust; `test/main/view-split-test` passed 4/4 in each, with original assertions unchanged. `rust/tests/pane-layout.py` passed 17 C/Rust comparisons. Real slow-Git PTY checks passed for split cancel, success, error retention, focus switch, resize, close, and terminal exit; split `R/z` responded in 0.001–0.002 s while the Git wrapper would otherwise sleep 30 s. An independent read-only review found a PTY assertion could accept a prior frame; that assertion was tightened and the PTY check passed again.
