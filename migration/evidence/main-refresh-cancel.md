# Main refresh cancellation — bounded receipt

Tested source: `deee688500f49d317887ff5727ad6e7b44aedcc1`, rebased onto `44ad2139a4f029a7639648d15f6db41d9913ea98` (PR #68 integrated without conflicts).
Host: Darwin arm64; no dependencies or unsafe code added.

## Scope

Interactive standalone main history (no associated parent/child pane): `R` (the `refresh` binding) starts Git log
without blocking terminal input. `z` (`stop-loading`) kills and waits for that
child. Both pipes drain concurrently; completion is polled by the UI with
`try_wait`. Completed rows remain displayed until the replacement succeeds.
Movement/search use the retained rows; actions that can change context cancel
the pending job. Quitting and terminal-error unwinding also reap it.

This is **not complete stop-loading**. Initial loading, split panes, scripted,
prompt-command and periodic refresh, generic streaming, synchronous Git
preflight (including unborn-HEAD detection), and post-log decoration/status
queries remain follow-up work. Parsing/rendering after log completion is also
synchronous. Cancellation owns the direct Git child, not arbitrary descendant
process trees; readers never block the UI waiting for a descendant's pipe EOF.
Existing Git argv construction, parsing, decoration and row rendering are reused.

## Checks

- `cargo fmt --check`, `cargo test --locked`,
  `cargo clippy --locked --all-targets -- -D warnings`, and release build: pass.
  Tests: 96 library + 25 application + 2 integration = 123; zero failures.
- `python3 rust/tests/main-refresh-pty.py target/release/tig`: pass.
  The wrapper fills stderr with 256 KiB and writes 1024 copies of history output before
  announcing its PID, proving both pipes drain. It then sleeps for 30 seconds.
  Navigation leaves that child alive. `z` responds in 0.082 s (2 s deadline),
  PID is gone, retained old rows remain and pending new rows never appear.
  Next `R` succeeds; exit-23 refresh retains the last successful rows;
  `Q` during another delayed refresh reaps its child.
- Negative baseline: built unchanged `9b7fbe073f8d8769a04c0f656eb28e4d498566e0`.
  Same focused test (before the navigation assertion was added) exits 1:
  `AssertionError: (b'Loading stopped', ...)` after the 2-second deadline.
  Git wrapper had announced its PID; the UI remained in synchronous history loading.
- Latest-main options-menu PTY integration: 10/10 cases pass.
- PR #68 integration: `python3 rust/tests/file-blame.py` passes all 31 checks;
  tracked index/worktree remain unchanged.
- Unmodified original scripts, C and Rust paired: **7/7 scripts, 29/29 assertions
  each**, no skips or unreached assertions. No full 154-script run claimed.

Reproduce the pair:

```sh
python3 rust/tests/upstream-suite.py test/main/default-test test/main/refresh-test test/main/refresh-periodic-test test/main/graph-argument-test test/main/main-options-test test/main/view-split-test test/blame/blob-blame-test --output work/main-refresh-pair.json
```

Independent read-only review before this rebase: no material findings.
This conflict-free rebase was inspected and all checks above rerun on the combined source.
Earlier findings about in-view navigation and maximized child views were fixed.

## SHA-256

- Rust executable: `6ea896a5135c6c89b0ad9a3d7e260d2536540ceb7dd9999035fc6cb548095ab8`
- C executable: `16dbbda87e0619f32cb959bd6dd60725d2f4657c7773243d283d42fd3bb52345`
- Negative baseline executable: `b7a0724d7ffb26c83b68bc9a68c9f57d4325d882a064cb05ad86a30a42962974`
- `rust/git.rs`: `78a8f085553a2321be26a0237d49333cb62f9937e43951fb781325385461d17c`
- `rust/main.rs`: `6a5f34be14426573f4a170d81582c50a2ff68a7c7e3ce83a98b40ff16b1afcaa`
- Focused PTY check: `45b1f5fe8159484611ca6e21d5759f9fec389be0bf71ebba29fb6681ad8210a1`
- Strict original-script runner: `8070849f857a5cd5a8d42b519c091eaa87c39a43048daca702abd163f74cf702`

One Markdown receipt is retained; generated paired JSON and build logs stay in
local scratch storage. Original C tests and their assertions are unchanged.
