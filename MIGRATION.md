# Migration status: IN PROGRESS / parity gate OPEN

This checkpoint is **not a completed Tig migration**. The user chose safe,
idiomatic Rust over a mechanical unsafe translation. Original upstream C files
remain unchanged as the reference and migration backlog. No production-ready
or full application parity claim is made.

Upstream: https://github.com/jonas/tig
Pinned source: `7d841c9302456b8c8f0629a559344137dc4faa4f`
License: GPL-2.0-or-later; original history, COPYING and copyright notices retained.

## Current implementation

- `rust/graph.rs`, `rust/graph_v1.rs`: owned v2 and v1 graph state/canvases,
  ASCII and Unicode output. Default terminal commit markers follow curses.
- `rust/git.rs`, `rust/model.rs`: Git subprocess backend and owned parsed records;
  history filters, refs, tree, blame, diff, status and whole-file staging.
  Returned filenames use lossless Unix paths; modifying commands use literal
  pathspecs and do not invoke a shell.
- `rust/patch.rs`: byte-preserving text hunk/line staging and reverse unstaging,
  checked index-only application, explicit rejection of unsupported patch forms.
  Canonical diff path prefixes are forced and regression-tested against user
  `diff.noprefix` settings to avoid modifying a similarly named wrong path.
- `rust/render.rs`: configurable main columns, widths, author/committer metadata,
  reference formats and common date formats. Local/relative dates remain
  unsupported.
- `rust/config.rs`: configuration/CLI parsing, include diagnostics, bindings,
  validated scoped column/global toggles and argument-list updates.
  Retaining a setting is not equivalent to implementing its visual effect.
- `rust/commands.rs`: argv-based external commands, explicit confirmation,
  selected-reference validation, foreground controlling-terminal streams,
  output acknowledgement, quick commands and first-line stdout echo.
  Unknown or unavailable selection variables fail explicitly. No implicit shell.
- `rust/main.rs`: initial terminal application using Crossterm, with owned view
  state and terminal cleanup, split panes, parent/child focus/navigation,
  branch/tracking status headers and status position restoration.
  It does not call the original Tig binary.

First-party Rust uses `forbid(unsafe_code)` through the crate and Cargo lint.
This does **not** mean dependencies, the OS or Git are unsafe-free. Crossterm,
signal-hook and their platform dependencies encapsulate system interactions.
The terminal implementation is now Crossterm, rather than the feasibility
report's initial proposal to retain curses; this avoids handwritten unsafe FFI
but creates a larger terminal-compatibility verification obligation.

## Evidence and verification

- C baseline: 572 assertions passed, 152 tests executed, 2 skipped. The ordinary
  shell inherited `init.defaultBranch=main`, causing 57 failures; rerunning with
  a command-scoped `master` override passed without changing assertions.
- The skipped original tests require diff-highlight and an address-sanitizer
  build respectively. See `migration/evidence/c-baseline.json` and full logs.
- Rust unit/fixture tests, bytewise graph differential comparisons and real PTY
  tests are independent checks. They are **not** a substitute for the 154-file
  upstream application suite.
- v2 graph evidence excludes curses attributes and the GH490 main-view fixture.
  Separate v1 evidence covers glyphs, color IDs and merge flags: 4,084 bytewise
  comparisons. It does not prove terminal color rendering.
- PTY evidence covers only the workflows listed in its JSON; it does not prove
  all terminal combinations or platforms work.

Reproduce local checks:

```sh
cargo fmt --all -- --check
cargo test --locked --all-targets
cargo clippy --locked --all-targets -- -D warnings
cargo build --locked --release
python3 rust/tests/terminal-smoke.py
python3 rust/tests/graph-differential.py --c-binary /path/to/original/test/tools/test-graph --rust-binary target/release/test-graph
```

GitHub's Rust workflow runs tests on Linux and macOS. Local evidence was obtained
on macOS arm64. The first checkpoint `e0bcbbf4` passed Rust CI run 36254215202,
original Linux run 36254215159 and original macOS run 36254215213. These historical
results do not validate later commits.

## Remaining completion criteria

The following work is required before calling the migration complete:

1. Full upstream executable-test adapter and assertion mapping; rerun all 154
   original test files, account separately for skipped/environment failures,
   and mutation-check the judge. Current Rust UI snapshots are not identical.
2. Complete nested split/navigation semantics, incremental asynchronous Git
   loading/cancellation, refresh/watch behavior and all view-specific actions.
3. Partial-block selection, complex rename/mode/binary patch handling,
   conflict/revert flows, editing and complete configured external-command
   context, including selected-commit browsing variables. Unsupported actions fail explicitly instead of executing
   a different operation. Whole-file and ordinary text hunk/line staging are
   implemented; individual-line added/deleted-file patches are rejected.
4. Complete configuration effects, colors, columns, regex search,
   key sequences, mouse behavior and TIG_SCRIPT/TRACE semantics. Current search
   is literal, script support is a subset, and screen snapshots differ.
5. Full CLI semantics and path filtering. Non-UTF-8 command-line arguments are
   rejected with a diagnostic (tree/status model paths are lossless). Commands
   such as `show` do not yet honor every upstream option; no equivalence claim.
6. PTY signals/job control and long-running child cancellation on all supported
   platforms; memory/load limits. Git output currently buffers in memory.
7. End-to-end performance and memory comparison only after matching complete
   workloads and correctness. Component measurements cannot close parity.

The first full upstream-harness attempt is preserved in
`migration/evidence/rust-upstream-initial.json` and its log. It failed and ran
before column rendering and partial-stage integration. Its mixed helper routes
and early exits cannot be interpreted as a Rust test pass percentage.

First pushed checkpoint (`e0bcbbf4`) upstream-harness result: **571 of 647 assertions failed**,
150 tests reported and 3 skipped; see `migration/evidence/rust-upstream-suite.json`
for the attempted-target ledger and exact caveats. This mixed route still uses
the C graph helper, so even successful assertions are not all Rust results.
The migration is not complete.

A second full harness run attempted all 154 recipes and reported **462 of 643
assertions failed**, 150 tests and 3 skips. See
`migration/evidence/rust-upstream-second.json` and its raw log. That run predates
final command-context, named-goto, notes-toggle and locale fixes; its binary hash
is retained rather than relabelled as final-source evidence. Different assertion
counts reflect early exits, not a completion percentage. Final-source focused
and terminal checks are recorded separately.

No percentage-complete estimate is inferred from lines of Rust or passing unit
checks. The reference C implementation is retained until these criteria pass.

## Second checkpoint verification

`migration/evidence/second-checkpoint.json` binds the final source and binary to
29 unit tests, 103 PTY checks, and 18 unchanged assertions across the original
main-default, status-start-on-line, and status-refresh tests. Formatting and
Clippy with warnings denied pass. Graph v2 has 2,042 byte-identical comparisons;
v1 has 4,084 glyph/metadata comparisons. These scoped successes do not supersede
the failing full-suite receipt above. The status restoration case involving
only a horizontal scroll offset still needs upstream parity coverage.

The refreshed component benchmark is in `migration/BENCHMARK.zh-CN.md`.
