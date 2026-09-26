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
- `rust/refs_view.rs`, `rust/tree_view.rs`: reference and directory rows,
  metadata, columns, filters, sorting, annotated tags and recursive trees.
  Custom `TIG_LS_REMOTE` loading, exact reference sort ties and all mailmap/date
  configuration effects still need compatibility work.
- `rust/main.rs`: initial terminal application using Crossterm, with owned view
  state and terminal cleanup, split panes, parent/child focus/navigation,
  branch/tracking status headers and status position restoration.
  It does not call the original Tig binary.
- Search now uses the maintained `regex` crate for pattern matching, case
  options and optional wraparound. This is not yet a POSIX ERE compatibility
  claim; syntax and which hidden fields are searchable still need comparison.

First-party Rust uses `forbid(unsafe_code)` through the crate and Cargo lint.
This does **not** mean dependencies, the OS or Git are unsafe-free. Crossterm,
signal-hook and their platform dependencies encapsulate system interactions.
The terminal implementation is now Crossterm, rather than the feasibility
report's initial proposal to retain curses; this avoids handwritten unsafe FFI
but creates a larger terminal-compatibility verification obligation.

## Reuse decisions

- [rust-lang/regex](https://github.com/rust-lang/regex) is reused for search
  instead of writing a regex engine. Version 1.13.1 supports Rust 1.81 and is
  MIT/Apache-2.0 licensed. Original Tig uses POSIX extended expressions, so
  upstream search tests remain the compatibility gate.
- [GitUI](https://github.com/gitui-org/gitui) and
  [gitu](https://github.com/altsem/gitu) are useful Rust UI references, but
  their navigation and Git models are not Tig-compatible. GitUI still lists
  commit graph structure on its roadmap and requires a newer compiler; a
  wholesale fork would discard already tested Tig graph behavior.
- [gitoxide](https://github.com/GitoxideLabs/gitoxide) offers reusable pure
  Rust Git crates. Its current `gix` MSRV exceeds this project's Rust 1.81
  floor, and [blame status](https://github.com/GitoxideLabs/gitoxide/blob/main/crate-status.md)
  is incomplete. We will evaluate individual crates against a specific failing
  Tig behavior instead of replacing the existing Git subprocess backend at once.

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
4. Complete configuration effects, colors, columns, search parity,
   key sequences, mouse behavior and TIG_SCRIPT/TRACE semantics. Search and
   script support remain subsets, and screen snapshots differ.
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

## Third checkpoint verification

The latest local source adds log graph/pretty formats with commit context,
reference rows and browsing, directory metadata/columns/sorting, recursive trees,
and directory-parent selection restoration. View history retains its arguments.

34 unit tests and 103 PTY checks pass. Eleven unchanged original application
scripts pass 43 assertions across main, status, tree, refs and log; the raw receipt
is `migration/evidence/upstream-focused-third.log`. Tree fixture unit checks also
compare 15 original screen bodies. These are scoped results, not complete parity.

The original second-checkpoint component benchmark remains historical; it is
not an end-to-end benchmark of this newer UI. Full-port performance remains gated
on the unresolved compatibility requirements above.

The third full upstream run attempted all 154 recipes and reported **412 of 638
assertions failed**, 150 tests and 3 skips. See
`migration/evidence/rust-upstream-third.json`; it uses the same mixed C graph
helper route and predates the final log-header parser tightening. The final
binary has a regression test for indented commit-message text plus the focused
application/PTY checks. Do not infer a completion percentage from these counts.

## Fourth checkpoint verification

Search now uses `regex`, including Tig's case and wrap options and scripted
`/pattern<Enter>` input. The unchanged original `test/main/search-test` passes
all 8 assertions. Editor invocation, `%(lineno)` command context, scroll-line
actions and patch-row file/line selection are implemented with argv-safe file
paths. Three original editor scripts now pass 18 of 35 assertions; the remaining
screens still expose diff/stage and synthetic main-state differences.

The fourth full upstream run attempted all 154 recipes and reported **345 of
610 assertions failed**, 150 tests, 3 skips and 1 recipe with no result receipt.
See `migration/evidence/rust-upstream-fourth.json` and its raw log. Different
assertion totals reflect early exits, not a completion percentage. This remains
a mixed route with the C graph helper. This full run predates a final guard
against editing the prior commit's file from a later commit header; that guard
has a focused unit regression. 36 Rust unit tests and 103 real PTY checks pass
on the rebuilt binary. Rust full parity and end-to-end benchmarks remain gated
on the remaining failures.
