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

## Final hash receipt

- Base commit: `67978a7b71d1138ef5ca084b79e1bdf3c036377f`
- Source commit: `90c6b97b6606a0c2be881498e6f9b32d2ad8ecfc`
- `rust/main.rs`: `da083caf7aecec3d16613d37664ad0cca007dc335f61cf5af6968e50e8ade487`
- Rust release `target/release/tig`: `72659fef269ab0a6dca6aba76d5b7fa2d5b64fb3770176fd8f3aa5051ed5137c`
- C `src/tig`: `969c12e7b663c39686e88b71c774c35ec0923f6c1de430a8138179e35a9706b0`
- Original `test/tree/chdir-test`: `168a6399b9f4b02774f284993da4e389a3af6fce8313b61975ef94eab47ad475`

All listed hashes are SHA-256. The working source hash matches the `rust/main.rs` blob in the source commit; `cargo build --release` reported the tested release binary up to date.
