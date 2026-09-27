# Tree back/parent navigation

## Behavior

C routes tree `REQ_BACK` and `REQ_PARENT` to the parent directory and closes the view at repository root. Baseline Rust already handled back correctly when a saved parent tree existed, but back from a tree first opened at an invocation-directory prefix closed to `main`; root `parent` also left the tree open. Nested `parent` navigation dropped the saved horizontal offset. Rust now uses the existing tree-parent helper for non-root tree back/parent, closes at root, and restores the saved horizontal offset.

## Verification

- Rust 1.81 `cargo fmt --all -- --check`: pass.
- Focused Rust `first_tree_open_uses_invocation_directory_once` test: pass; covers saved-tree back, parent navigation, horizontal offset restoration, and root close.
- Rust 1.81 release build: pass.
- Unchanged `test/tree/chdir-test`: C pass, Rust pass via `rust/tests/upstream-suite.py`; original assertions unchanged. The paired runner uses a PTY and real Git fixture.
- PTY/Git root-parent repro: C `[main]`; unmodified Rust at `67978a7b` `[tree]`; changed Rust `[main]`.
- PTY/Git invocation-prefix back repro from `common/src`: C `[tree]` at `/common/`; unmodified Rust `[main]`; changed Rust `[tree]` at `/common/`.
- The full 154-script suite was not run, as requested.

## Remaining differences

None observed in the selected C/Rust tree-navigation cases. The full paired suite was not run, so this does not establish repository-wide parity.
