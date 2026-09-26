# Trace and status migration checkpoint

Base: `2fb2a871` (latest main when this independent clone was created).
Implementation and verification source: `d351cb6506d0799ba88d502122e7caa0203d9e2d`.
No AGENTS.md was present in the clone or its ancestor directories.
The C source, original scripts, assertions, Makefile, and upstream adapter are unchanged.
First-party Rust still forbids unsafe code; no dependencies or Actions changes were added.

## Behavior

`TIG_TRACE` now appends the actual child program/argv and captured stderr.
Repository queries, grep, user commands, patch application and native date formatting
use this tracing. A missing/unwritable trace destination never prevents execution.
Tracing does not synthesize C commands, expected files, or test receipts. Interactive
commands retain their terminal stderr; only captured stderr is appended.
The repository runner expresses pager, literal-path and color policy through Git's
environment equivalents and preserves inherited `GIT_CONFIG_COUNT` entries.

The status view uses actual NUL-delimited `diff-index`, `diff-files` and `ls-files`
results, applies the path filter, and honors `status-show-untracked-files`. Unborn
branches use cached filenames. Rename source/destination bytes remain separate;
malformed/truncated records and unsafe repository-relative paths are rejected.
Unmerged worktree entries are collapsed as in C. The existing porcelain query remains
in use for synthetic main changes and mutation lookup.

An untracked stage pane now has the correct title and advances to the next status
entry after staging. Empty untracked panes do not stage a file; the status pane still
can. Refresh preserves the filter context. Path-filtered history now requests
`--parents`, so Git rewrites links across omitted commits instead of leaving dangling
graph lanes. The three-commit regression failed before adding this option and passes
with it.

## Verification

All evidence below is from macOS arm64 and is limited to the selected tests.

| Four requested scripts | C assertions | Rust OK | Rust FAIL | Rust NOT_REACHED |
|---|---:|---:|---:|---:|
| Before, `trace-status-before.json` | 20 | 7 | 6 | 7 |
| After, subset of `trace-status-after.json` | 20 | 17 | 3 | 0 |

Both `status/file-name-test` and `status/untracked-files-test` pass in full. All
screen assertions in the four requested scripts pass. `status/file-filter-test`
also passes the real diff-index, diff-files and ls-files trace assertions and its
original no-update-index guard.

The three remaining trace assertions deliberately fail: main/status rev-parse argv
are different because Rust discovers and validates the repository differently;
main log argv differ because Rust uses a different metadata format. The traces
expose those differences faithfully. They are not equivalent-receipt passes.

The expanded paired run covers all 12 status scripts, all five stage scripts,
main default/all-arg/filter-args, and one original graph script. C: 21 scripts,
120 assertions pass. Rust: 17 scripts pass, four fail; 111 assertions pass, five
fail, four are not reached. The adapter exits 1 and records `BLOCKED`.
Besides the two trace-dependent scripts, remaining failures are the existing
`main/all-arg-test` unsupported `revargs` command variable and the existing
`stage/split-chunk-test` hunk-count difference (`+8,2` versus C's `+8,1`). The latter
matches the earlier `stage-parity/paired-rust-only.json` failure. Neither is hidden.

`evidence/trace-status/checks.json` records actual subprocess exit codes and log
hashes for fmt, all 72 unit tests, and Clippy with warnings denied. All pass.
`evidence/trace-status/pty.json` records 130 passing checks from the unchanged
terminal smoke runner, with only its output destination redirected. Its binary
hash matches the Rust application in the paired report. Unit checks cover real
trace appends/failure status/unwritable destinations, filtered rename paths and
index preservation, malformed raw records, and filtered parent rewriting.

Reproduce the paired run:

```sh
python3 rust/tests/upstream-suite.py --output migration/evidence/trace-status-after.json \
  test/status/*-test test/stage/*-test test/main/filter-args-test \
  test/main/default-test test/main/all-arg-test test/graph/00-simple-test
```

This is not a full-suite rerun or a claim that Rust parity is complete.
