# Stage, blob and status blame navigation

Base: `580d65f4acf7597f8b55fdb933218d12746e149a` (rebased without conflicts after options-menu PR #67; all listed checks rerun on the combined source). Modified source is bound by the SHA-256 values below. Rust MSRV 1.81, `forbid(unsafe_code)`, GPL and upstream attribution are unchanged.

Before: these views used generic blame dispatch, losing the selected blob/stage line and retaining unrelated revision context. After: blob uses its displayed revision/source row, status uses the selected tracked worktree file, and stage uses diff paths/lines. Deleted stage lines resolve to HEAD, accounting for staged offsets and renames. Refresh and close preserve the selected source position.

Validation (macOS arm64; no 154-script run):
- `rustup run 1.81.0 cargo build --release --locked`: pass; `rustup run 1.81.0 cargo test --locked --bin tig`: 25 pass, 0 fail/skip.
- `cargo fmt --check`; `cargo clippy --locked --bin tig -- -D warnings`; `git diff --check`: pass.
- `python3 rust/tests/upstream-suite.py test/stage/default-test test/blob/wrap-lines-test test/status/worktree-test test/blame/blob-blame-test --output ../focused-upstream.json`: 4 C + 4 Rust passes, 0 fail/skip; original assertions unchanged. The runner rebuilds the release executable using the default Cargo toolchain; its tested hash is below.
- `python3 rust/tests/file-blame.py`: 31 executions pass: 14 paired C/Rust scenarios, 2 Rust safety checks, 1 C safety-difference baseline. Covers historical/worktree blob, current row, stage stats/additions/deletions, staged offsets/rename, disabled status rename detection, header-like deleted content, headings/untracked files, refresh/return, unchanged tracked worktree/index.
- Two fresh-context independent read-only reviewers: no remaining findings after fixing deleted-header-like text, cached blob attribution, and config-dependent rename lookup.

Safety differences: a deleted index-only addition has no committed source. C selects unrelated HEAD line 1 in the recorded fixture; Rust stays in stage with an error message. Cached grep-origin blobs are explicitly rejected rather than blaming unrelated worktree coordinates. Missing paths/history and unsupported patches leave the source view usable. These are not full parity claims.

SHA-256 (base plus these three source files identifies this slice; binaries were executed above):
- `rust/main.rs`: `be1c72cdaa8b0823ee492901f7397038498c8854af77ee871124f1293af2d026`
- `rust/git.rs`: `5846c61ca6619098b5c8b9f196068fa63f6552554ebe0a43870b123c306fe9be`
- `rust/tests/file-blame.py`: `d79f4109dd199698ae74cb7136041e153c322541849f37679ff5eb6d4ef671c1`
- `target/release/tig`: `12dbe45a2d5a024e9f475c69a2b6110b3071dffe0606d6fe476ca254555b632b`
- `src/tig`: `16dbbda87e0619f32cb959bd6dd60725d2f4657c7773243d283d42fd3bb52345`
