# Main-view merge navigation

Base: merged `main` at `39b9ca6a13c1e79a0e5208dca984764d1a64e7e7`. Rust now handles `move-next-merge` and `move-prev-merge` using owned commit parent records and the existing `wrap-search` setting. Before this slice those advertised requests had no main-view action.

Verification: Rust's existing real Git main-view unit test now checks next, previous, wrap, and no-wrap; formatting, Clippy with warnings denied, and release build pass. Unchanged original `test/main/merge-test` and `no-merges-test` pass on C and Rust (2 reached assertions each). A separate real-PTY scripted `move-next-merge` plus `save-view` check in a repository with two merges selected line 3 in both C and Rust. The original tests do not directly assert the new action. The complete 154-script gate remains the prior integrated checkpoint.

SHA-256: `rust/main.rs` `691fcb12545cea865630de023125a205c1116400239cb3d6205d45b40259affc`; `target/release/tig` `e97828a0d3819a3ffc9a9709f28622eb2dc03b78f521ba2356f6570ae11f73eb`.
