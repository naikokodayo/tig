# Initial `show REV -- pathspec` filter

Base: `27edc2b71e02ff042b665b5379235e2dd805cafb` (`origin/main`). Code: `8df4a9d80afc84fc20424589ed49a711f184ad56`.

Before: Rust displayed the full commit for `tig show HEAD -- wanted.txt`; C displayed only `wanted.txt`. After: the initial Rust diff view forwards CLI pathspec arguments as separate Git arguments after `--`, while ordinary diff navigation keeps its existing internal file filter. The CLI filter survives `:refresh`. No shell interprets pathspecs. An independent read-only review identified the `.` pathspec boundary, which is now covered by the regression. The reviewer also noted unseparated forms such as `show HEAD wanted.txt`; those are outside this `REV -- pathspec` slice and need the wider CLI argument split.

Checks: Rust 1.81 `cargo fmt --all -- --check`, `cargo test --all-targets` (97 library, 26 application, 2 watch), `cargo clippy --all-targets -- -D warnings`, and `cargo build --release` passed. `python3 rust/tests/show-pathspec.py` passed against both C and Rust for one/two files, an option-like filename, a renamed filename with spaces, a shell metacharacter filename, and `.`; every case preserved the real Git index and worktree.

Unchanged original scripts: `diff-context-test`, `diff-stat-test`, and `diff-stdin-test` passed in C and Rust. `filter-args-test` passed in C; Rust passed `filtered.screen` but failed `rev-parse.trace` and `log.trace`. Those same two Rust trace failures are present in `upstream-rust-only-current.json` at base `6f029183b43d166271526e45d8f227ab430ce073`; this slice did not change that CLI trace behavior. The strict paired gate remains blocked.

SHA-256:

| Artifact | Hash |
| --- | --- |
| `rust/main.rs` | `e1bb3e1c34522325140159299934a6ae78cf0ee1409de8b469fb4439ff385366` |
| `rust/git.rs` | `7f8756273d7f555d8fb3cf29fe80f3b81eccaba039163e9fa06d3b7a37564bef` |
| `rust/tests/show-pathspec.py` | `e294b8f717c91ae357058f81c79f2d258f5404afd99348e9fdad3617a61de0d6` |
| `src/tig` | `969c12e7b663c39686e88b71c774c35ec0923f6c1de430a8138179e35a9706b0` |
| `target/release/tig` | `f5bdf905e61ac7c781ad11d9072ce6ec98c60243287fa3055b0da5d45fe60a3d` |
| `test/diff/diff-context-test` | `70d66cc7fbbace58b14cf4c12613de5bc00f084e3ed9d043e2b886daad275dd4` |
| `test/diff/diff-stat-test` | `2e3504a253b87cb00c42babbf7014ca384fbe2951bba56232a10ef5a688e151d` |
| `test/diff/diff-stdin-test` | `254d04fcf06b75967b9d47904b78110ef7c8f0b211609a345093549b35a18568` |
| `test/main/filter-args-test` | `71a55ff435f3cd25168a004233b7ea6c34e71137c9483a91e921aaae49e3c9c4` |
