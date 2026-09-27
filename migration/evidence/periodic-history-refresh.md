# Periodic main history refresh receipt

Base: `b26209af739b1edb584864cdef0c61efb634cbaa` (`origin/main`). In the single-pane main view, a periodic change now starts the existing cancellable `HistoryRefresh`; old rows remain during loading, `z` cancels and reaps Git, and a failed load marks the watch for retry. A disposable slow-`git log` PTY probe timed out waiting 1 s for `z` before this change; afterward `z` responded in 0.001 s with a Git command configured to sleep 4 s. The same probe verified automatic retry after a failed history command without a further repository change. It was kept outside the repository, so this PR adds no Python.

| Source or binary | SHA-256 |
| --- | --- |
| `rust/main.rs` | `f62617d5a0435180d91537a35b4bcd5706f7429fdc8ae1b0ae878ab008b8c35f` |
| `test/main/refresh-test` | `299ee5ce1fc8fd738059aa30604c50411c876e8c0a7035684056f85b94b79111` |
| `test/main/refresh-periodic-test` | `f7d8f6eb6ae2ba733d210d52f5140d14ce23a765961fa8a8c561dfe257e5446c` |
| `src/tig` (C) | `54db05e55837e0e9ea34d91b08957faca549285e437683cd31fc504f222b6dd1` |
| `target/release/tig` (Rust) | `5c95861f49de9ddc7774bf4e359659c061b75d52b663b9d2727178af7a1cdf23` |

`cargo fmt --check`, `cargo test` (125 tests), `cargo clippy --all-targets -- -D warnings`, and `cargo build --release` passed after rebasing. Original C/Rust scripts passed unchanged: `refresh-test` 8/8 and `refresh-periodic-test` 5/5 assertions in each binary. Existing `watch-pty.py` passed all three refresh policies; `main-refresh-pty.py` passed all checks.

Independent read-only review found no actionable issue in this slice.

Remaining synchronous paths: `Watch::poll` Git snapshots (`for-each-ref`, status, diff), multi-pane periodic refresh (a diff pane may itself call `git log` for metadata), cached parent refresh, after-command refresh, and initial loading. These require a separate lifecycle change to make cancellable without changing other panes' update semantics.
