# Trace and status migration checkpoint

Base: `2fb2a871` (latest main when this independent clone was created).
Final code/verification source: `b21df5669259b90b6f785273c42117678f36b357`,
after both review fixes and synchronization with main `c097ffb8`.
The following evidence-only commit does not alter that source tree.
Initial checkpoint evidence at `d351cb65` is historical, not PR-head verification.
No AGENTS.md was present in the clone or its ancestor directories.
The C source, original scripts, assertions, Makefile, and upstream adapter are unchanged.
First-party Rust still forbids unsafe code; no dependencies or Actions changes were added.

## Behavior

`TIG_TRACE` now appends the actual child program/argv and captured stderr.
Repository queries, grep, user commands, patch application and native date formatting
use this tracing. A missing/unwritable trace destination never prevents execution.
Tracing does not synthesize C commands, expected files, or test receipts.
Foreground interactive commands are excluded entirely, matching C’s `IO_FG`
boundary; their argv and terminal stderr are not written to the trace.
The repository runner expresses pager, literal-path and color policy through Git's
environment equivalents and preserves inherited `GIT_CONFIG_COUNT` entries.

The status view uses actual NUL-delimited `diff-index`, `diff-files` and `ls-files`
results, applies the path filter, and honors `status-show-untracked-files`. Unborn
branches use cached filenames. Rename source/destination bytes remain separate;
malformed/truncated records and unsafe repository-relative paths are rejected.
Unmerged worktree entries are collapsed as in C. The existing porcelain query remains
in use for synthetic main changes and mutation lookup. The same active filter now
reaches aggregate stage/unstage diffs on both initial opening and refresh, rather
than only hiding status rows. Explicit single-file paths retain strict validation;
query filters are literal argv after `--`, and patch application retains its
existing validation and index-only safety checks.

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
main default/all-arg/filter-args, prompt exec, and one original graph script. C: 22
scripts and 124 assertions pass. Rust: 18 scripts pass, four fail; 115 assertions pass, five
fail, four are not reached. The adapter exits 1 and records `BLOCKED`.
Besides the two trace-dependent scripts, remaining failures are the existing
`main/all-arg-test` unsupported `revargs` command variable and the existing
`stage/split-chunk-test` hunk-count difference (`+8,2` versus C's `+8,1`). The latter
matches the earlier `stage-parity/paired-rust-only.json` failure. Neither is hidden.

Latest validation is in `evidence/trace-status/review/`:

- `checks.json` records actual subprocess statuses and hashes: fmt, **74 unit
  tests** (55 library + 19 application), Clippy with warnings denied, and filtered
  index checks all pass.
- `pty.json` records **131 passing controlling-PTY checks**, including the new
  trace boundary regression: captured/silent command argv remain traceable;
  normal foreground, successful quick, and failed quick argv are absent.
- `foreground-pair.json` independently checks real interactive C and Rust:
  the foreground command executes without tracing its synthetic private marker;
  the background command executes and is traced.
- `status-filter.json` contains **10 passing real C/Rust index checks**: aggregate
  staging, staging after refresh, unstaging after refresh, multiple filtered paths
  (including a space), and intentionally disabling the filter. Outside-filter
  index content and all worktree file bytes are checked, not just screen text.
- `pty-red.json` / `pty-red.log` preserve the pre-fix foreground leak;
  `status-filter-red.json` preserves C passing all five scenarios while Rust
  incorrectly changed the outside index in four. These are negative evidence,
  not passing receipts.

The latest PTY, index, and foreground-pair binary hashes match the Rust application
in the updated `trace-status-after.json` (source `b21df566`). Older `checks.json`
(72 tests) and `pty.json` (130 checks) outside the `review/` directory retain their
historical `d351cb65` evidence only. They are not used to certify the synchronized
PR. Original C tests/assertions and the upstream adapter remain unchanged.

Reproduce the paired run:

```sh
python3 rust/tests/upstream-suite.py --output migration/evidence/trace-status-after.json \
  test/status/*-test test/stage/*-test test/main/filter-args-test \
  test/main/default-test test/main/all-arg-test test/graph/00-simple-test test/prompt/exec-test
```

Reproduce the index regression after building both binaries:

```sh
python3 rust/tests/status-filter.py --output migration/evidence/trace-status/review/status-filter.json
```

This is not a full-suite rerun or a claim that Rust parity is complete.
