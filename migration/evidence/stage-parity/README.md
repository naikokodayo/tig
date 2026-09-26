# Stage and partial patch compatibility slice

Status: scoped improvement; full Rust migration/parity remains open.

The branch began at `3e8f4b8` and merged `34684f7` before the final checks.
Implementation source at verification: `cf6fda4089691d07a83aea4a136c1db8a95b6f57`.
All work used the independent `stage-parity-task-clone`; the default dirty checkout
was not edited. No upstream `test/` file was modified.

## Root causes and changes

- `stage-update-part` and `stage-split-chunk` had no Rust action handlers.
  Contiguous change blocks now reuse the existing line/hunk selector, including
  its reverse direction, file restrictions and no-newline marker handling.
  Splitting shares the intervening unchanged context and changes the displayed
  patch only. A selected split hunk still passes through the validated index API.
- `view-stage` did not retain a status row's staged/unstaged identity, and status
  section rows could not open an aggregate patch. Both now use the Enter flow.
- After applying a patch, only main parents were refreshed. A status parent could
  display the old index after the empty stage view closed (the gh-410 symptom).
  Refresh now handles main/status parents and preserves the relevant file/group
  selection, including a maximized stage shortcut.
- Stage stat rows now retain the Enter hint/navigation for single-file patches;
  `view-diff` maximizes an existing stage view.

`canonical_path`, `validate_apply_paths`, `apply_once` and `apply_cached` retain
all existing path validation, Git preflight, and atomic index update behavior.
There are no worktree writes, new dependencies, unsafe Rust, force/reject flags,
or relaxations for rename, copy, mode, binary, combined or added/deleted line
selection. All callers of the patch selection methods were inspected.

## Verification

On macOS arm64, Rust 1.81.0:

- `rustup run 1.81.0 cargo fmt --all -- --check`: pass.
- `rustup run 1.81.0 cargo test --locked --all-targets`: 65 tests pass.
- `rustup run 1.81.0 cargo clippy --locked --all-targets -- -D warnings`: pass.
- `rustup run 1.81.0 cargo build --locked --release`: pass.
- `python3 rust/tests/terminal-smoke.py`: 126 real PTY checks pass. Added checks
  exercise split, block stage/unstage, refreshed parent state, Git index contents,
  unchanged worktree contents and terminal restoration.
- Local original C executable: all five unchanged `test/stage/*-test` scripts
  pass (26 assertions).
- Rust: those five scripts pass 25/26 assertions. `default`, `update-part`,
  `gh-410` and `maximized-unstaged-changes` pass completely. The single remaining
  `split-chunk` screen difference is intentional and documented below.

The original four priority-script baseline failed 24/26 records (24 screen
assertions plus two unexpected exits for the missing commands). The new parent
refresh regression also failed against the original Rust implementation; see
`regression-before.log`. Unit coverage includes forward/reverse contiguous blocks,
no-newline markers, each split hunk applied through Git, existing path rejection
and failed-apply index preservation.

Original harness environment: release directory first in PATH,
`GIT_CONFIG_COUNT=1`, `GIT_CONFIG_KEY_0=init.defaultBranch`,
`GIT_CONFIG_VALUE_0=master`, and a real controlling terminal/stdin. The related
run covers all five stage scripts, all twelve status scripts, and main's
`update-unstaged-changes-test` and `untracked-test`. The related run records 98 passing and 7 failing assertions: one intentional
stage difference and six existing status failures, with no new failures in the
related scripts. See `receipt.json` for exact
per-script outcomes, baseline comparison and hashes. A recipe error in the
pre-existing status file-filter test prevents Make's final summary, so the
receipt counts the actual `.test-result` records without treating that error
as a pass.

## Deliberate split-chunk difference

The C split loop stops when it encounters the first no-newline marker, before
counting the following `+8`. Its final displayed header is:

```
@@ -8,4 +8,1 @@
 7
-8
-9
-10
\ No newline at end of file
+8
\ No newline at end of file
```

The new side has two lines (`7` and `8`), so Rust emits `+8,2`. Its parser continues
to reject a mismatch between hunk counts and body rows. On this Git version,
applying the C last hunk to a disposable index succeeds but loses the final `8`;
applying the corrected hunk keeps it. `split-header-check.log` records both exact
index contents. Neither check changes the real fixture index or worktree.
The unchanged upstream screen expectation is not adjusted to hide this difference.

## Remaining limitations

This is not a full-suite or full-migration completion claim. Existing status
path filtering, untracked-file title/navigation and untracked-files display
failures remain outside this slice; the related receipt compares their counts
to the prior checkpoint. Git can reject a whole aggregate patch containing
multiple split hunks with overlapping context; individual split hunks work, and
refreshing before staging the entire patch restores the unsplit patch. The
rejection remains atomic. Existing binary/combined/path restrictions remain.

Review was performed inline against the C stage/status flow and every patch
selection caller; no independent agent review is claimed. The original tests
remain available as failing evidence where compatibility is intentionally open.
