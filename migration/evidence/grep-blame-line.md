# Grep hit to blame line

Integrated with main `ae4d6a34` (PRs #65 and #66); the feature remains a 15-line addition.
C `src/grep.c` passes the hit line to blame. Rust now reuses the hit's validated
path/revision, opens working-tree blame for an unqualified grep, and positions
and reveals the source line. Existing unsupported revision/path boundaries remain.

- Before: C selected historical line 43; Rust selected line 1. Full baseline
  failure and initial receipt remain in commit `ab739dcb`.
- After integration: `python3 rust/tests/grep-blame.py` passes 10/10 C/Rust runs:
  historical/HEAD/worktree lines 43/44/45, header line 1, context line 42;
  refresh and return preserve position. Real Git/PTY; no tracked-file changes.
- Original scripts pass 12/12 and 50 assertions for each C/Rust implementation:
  `test/grep/{default,refspec,start-on-line}-test`,
  `test/blame/{default,start-on-line,blob-blame,initial-diff}-test`,
  `test/diff/wrap-lines-test`, `test/stage/default-test`,
  `test/log/{diff-stat,pretty-format}-test`, `test/main/show-changes-test`.
  Run these explicit paths with `rust/tests/upstream-suite.py`; no skips or weakened assertions.
- `python3 rust/tests/diff-origin-blob.py --output /tmp/diff-origin.json`:
  integrated diff/stage origin regression passes 8/8 per implementation.
- Rust 1.81.0 release build, 25 application tests (`cargo test --locked --bin tig`),
  formatting and application Clippy pass. Initial pre-integration 123 tests passed.
- Two initial and two integration independent read-only reviews found no material issues.

Source SHA-256: `f3e316afd9513404067b1e9380501cb3766b82e415320c2ee2d46561884682af`
(sorted tracked Cargo.toml/Cargo.lock/Makefile/tigrc/rust/src/include/compat/test
paths, hashing path bytes + NUL + contents + NUL).
Release `tig` SHA-256: `b7a0724d7ffb26c83b68bc9a68c9f57d4325d882a064cb05ad86a30a42962974`.

Scoped evidence only; no full 154-script run or completed migration claim.
