# Status file revert

In the status view, select one unstaged regular file and press `!` (or enter
`:status-revert`). Confirm with `y`, `Y`, or `yes` followed by Enter. Empty
input, Escape, and other answers cancel without writing. Scripted requests
always cancel; a script cannot supply consent.

For a conflict, use `:status-revert ours` or `:status-revert theirs`. A missing
side explicitly means deletion. The worktree changes but the conflict index
stages remain intact. Inspect the result, then use `u` / `status-update` to
mark it resolved, including deletions.

Before replacement, the original file is moved into a private directory under
`git rev-parse --absolute-git-dir` + `/tig-revert/`; its `README` identifies the
path, and `index-entries` records the original index stages. The `worktree`
file retains the old bytes and mode. Recovery directories are not automatically
removed. A failed checkout reports the recovery path; restore its `worktree`
file manually after inspecting the current target. No index write occurs.

Preparation and execution reject changed index/worktree snapshots, symlinks,
nonregular files, staged selections, untracked files, and nonrelative paths.
`checkout-index` runs without force. This is not an atomic transaction against
concurrent parent-directory replacement or Git index writers; external mutation
during execution is outside this slice's concurrency guarantee. Chunk revert,
symlinks, submodules, and automatic mergetool invocation remain unsupported.

For an unmerged status row, `M` / `:status-merge` launches the configured Git
mergetool after explicit confirmation. Tig restores the terminal and reloads
status afterward. Scripted commands cannot confirm this external operation.

Validation: five real-Git backend tests plus `python3
rust/tests/status-revert.py target/release/tig` exercise the actual terminal.
The same probe's `--c` option covers ordinary upstream confirmation/cancellation.
Original status filename, refresh, and worktree scripts remain unchanged.
