# Stage hunk revert

Base: merged `main` at `8cc3d374a9283be94adb2c075f781ac833b19a0a`. The Rust stage view now confirms and reverses one selected unstaged text hunk in the worktree, including a hunk split in the UI. The original Git index bytes remain unchanged. Before this slice, the `status-revert` action rejected the stage view; the C implementation routes that action through `stage_revert` in `src/stage.c`.

Verification: Rust 94 library, 24 binary and 2 documentation tests pass; formatting, Clippy with warnings denied, and release build pass. The unchanged original `test/stage/default-test`, `split-chunk-test`, and `update-part-test` pass on both C and Rust (21 reached assertions per side). Fourteen real terminal checks pass, including per-file and aggregate stage views, selected and split hunks, cancellation, stale worktree refusal, index preservation, and a recovery copy. Two independent read-only review rounds found and resolved split-hunk and aggregate-view path gaps; final review found no P1/P2 issue. The complete 154-script gate is deferred to the next integrated checkpoint.

SHA-256:

- `rust/main.rs`: `7e35e5536a64ce2c54a190c4826e4a541ca6934fd5a26f09571e40a10a35ea9b`
- `rust/patch.rs`: `255874e94a4ffa35c71c2233083ae0d3018e675687a0a948dcebcbd550f86e46`
- `rust/status_ops.rs`: `5e5f34a7a66387ec44cf14775e7f141262f01ab58fd5388dacbf303a3be4ef7b`
- `rust/tests/status-revert.py`: `939baf471d4367f0ce06e21bb7a11462789d9fe068a78f09673ac7328c124127`
- `target/release/tig`: `ca08fb3dc1e344057e02a8f919d37dab764015eaa46d96c77306724716427bd8`

Boundary: only regular tracked text hunks with an exact current Git diff or freshly re-split hunk are accepted. Unsupported file types and changed content fail closed. C sources and original assertions were unchanged.
