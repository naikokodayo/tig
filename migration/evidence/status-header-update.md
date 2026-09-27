# Status header update — focused receipt

Base: `origin/main` at `a3c48ab9`. Select a status section header and run `:status-update`: staged files are unstaged; unstaged or untracked files are staged. The Rust update uses one checked Git index command with NUL-delimited, validated literal paths. Empty sections report `Nothing to update`.

| Source or binary | SHA-256 |
| --- | --- |
| `rust/git.rs` | `ac65417e40791a6dbbcd06218025bd5e8f51eb39ce0e51ae4a4a902767262357` |
| `rust/main.rs` | `a45bb0cdfdc0bfc94ba65855b002bf159304232df896fb475210ea28678877a9` |
| `rust/tests/status-header-update.py` | `3f6d2b6c71064c51669c4d6096c71e050ae23f9d8f4741cda739f128d0e64a9a` |
| C `src/tig` | `ca291ff2ccb34aeb6cdb1b1ad6c8ff71ab821de309cef2f8ac6c3b48a112ab25` |
| Rust `target/release/tig` | `0f7a1501f67d1da11415c4936cf8a7a8876a8d262377b677bfae46796ad513b8` |

`python3 rust/tests/status-header-update.py`: 10/10 real Git/PTY cases passed across C and Rust (unstaged, staged with rename, untracked, filtered, empty). Index blob content and worktree bytes were checked. `python3 rust/tests/status-filter.py --output /tmp/tig-status-header-filter.json`: 16/16 filtered-index checks passed. Rust formatting, release build, 125 Cargo tests, and Clippy with warnings denied passed.

Unchanged original scripts: C passed all four; Rust passed `test/status/file-name-test`, `test/status/untracked-files-test`, and `test/status/refresh-test`. Rust `test/status/file-filter-test` passed five of six assertions; its existing `rev-parse.trace` expects C's discovery command sequence, while Rust uses its own discovery commands. Its status screen, stderr, and other three Git traces passed. Focused runner output is `/tmp/tig-status-header-original.json` (outside the repository).

Intentional safety difference: for a staged rename, C's header action restores the old path but leaves the new path staged. Rust clears both paths, consistent with its existing per-file unstage operation. The PTY test checks each result explicitly.
