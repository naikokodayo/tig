# Grep hit to blame line

Integrated with main `eba0d6da` (PR #65); the feature remains a 15-line addition.
C `src/grep.c` passes the hit line to blame. Rust now reuses the hit's validated
path/revision, opens working-tree blame for an unqualified grep, and positions
and reveals the source line. Existing unsupported revision/path boundaries remain.

- Before: C selected historical line 43; Rust selected line 1. Full baseline
  failure and initial receipt remain in commit `ab739dcb`.
- After integration: `python3 rust/tests/grep-blame.py` passes 10/10 C/Rust runs:
  historical/HEAD/worktree lines 43/44/45, header line 1, context line 42;
  refresh and return preserve position. Real Git/PTY; no tracked-file changes.
- Original scripts pass 9/9 and 35 assertions for each C/Rust implementation:
  `test/grep/{default,refspec,start-on-line}-test`,
  `test/blame/{default,start-on-line,blob-blame,initial-diff}-test`,
  `test/diff/wrap-lines-test`, `test/stage/default-test`.
  Run these explicit paths with `rust/tests/upstream-suite.py`; no skips or weakened assertions.
- `python3 rust/tests/diff-origin-blob.py --output /tmp/diff-origin.json`:
  integrated diff/stage origin regression passes 8/8 per implementation.
- Rust 1.81.0 release build, 25 application tests (`cargo test --locked --bin tig`),
  formatting and application Clippy pass. Initial pre-integration 123 tests passed.
- Two initial and one post-integration independent read-only reviews found no material issues.

Source SHA-256: `d8275eb4ddebd7d48998f08d9de84c8b5a68240155d4f5936cdd49b50538f99d`
(sorted tracked Cargo.toml/Cargo.lock/Makefile/tigrc/rust/src/include/compat/test
paths, hashing path bytes + NUL + contents + NUL).
Release `tig` SHA-256: `9cdf78e9c52519e3631775380c8fab0d33e8e4101469357caad1a0945cb64815`.

Scoped evidence only; no full 154-script run or completed migration claim.
