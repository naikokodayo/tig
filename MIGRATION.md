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
  reference formats and common date formats. Local and relative dates share
  `rust/date.rs`; Chrono validates/converts dates and GNU/BSD `date` supplies
  system timezone/locale formatting. See the dated compatibility receipt below.
- `rust/config.rs`: configuration/CLI parsing, include diagnostics, bindings,
  binding assignment order, validated scoped column/global toggles and
  argument-list updates.
  Retaining a setting is not equivalent to implementing its visual effect.
- `rust/commands.rs`: argv-based external commands, explicit confirmation,
  selected-reference validation, foreground controlling-terminal streams,
  output acknowledgement, quick commands and first-line stdout echo.
  Unknown or unavailable selection variables fail explicitly. No implicit shell.
- `rust/refs_view.rs`, `rust/tree_view.rs`: reference and directory rows,
  metadata, columns, filters, sorting, annotated tags and recursive trees.
  Custom `TIG_LS_REMOTE` loading now feeds both refs rows and main decorations.
  Exact reference sort ties and all mailmap/date configuration effects still
  need compatibility work.
- `rust/help_view.rs`: live help rows from active bindings and upstream request
  descriptions, including section collapse and help search.
- `rust/main.rs`: initial terminal application using Crossterm, with owned view
  state and terminal cleanup, split panes, parent/child focus/navigation,
  branch/tracking status headers, status position restoration, and synthetic
  untracked/unstaged/staged rows before HEAD. Aggregate stage diffs and
  untracked-only status can be opened from those rows.
  Pager-family line-number columns and selected diff file titles now render
  without modifying raw patch rows. `:view-diff` on a synthetic change row
  reads the worktree conflict diff and refreshes it with its Git prefix setting.
  It does not call the original Tig binary.
- Search now uses the maintained `regex` crate for pattern matching, case
  options and optional wraparound. This is not yet a POSIX ERE compatibility
  claim; syntax and which hidden fields are searchable still need comparison.
- The grep view now parses NUL-delimited Git hits, groups results by file,
  supports configured widths, and opens worktree or revision-tree blobs at the
  selected line. Ambiguous revision/path forms fail closed; context and other
  output-changing Git grep options remain unsupported in this Rust view.

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
python3 rust/tests/date-compatibility.py
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

## Fifth checkpoint verification

The main view now shows working-tree change rows before HEAD and opens the
corresponding aggregate stage or untracked-only status view. Whole-repository
stage updates refresh the parent, empty stage views close, and aggregate diff
stat rows jump to their matching patch. Editor targets come from the selected
patch path. Tree line numbers now count the directory header like original Tig.

The three unchanged original main change-row scripts pass all 12 assertions;
the five tree-width cases in the original width script also pass their screen
and stderr assertions. Rust formatting, all 39 unit tests, Clippy with warnings
denied, release build, and 108 real PTY checks pass. An independent review
found no remaining high-priority wrong-file or index-mutation issue in this
checkpoint.

The fifth full upstream run attempted all 154 recipes and reported **316 of
609 assertions failed**, in 150 tests with 3 skips and 1 recipe without a
result receipt. See `migration/evidence/rust-upstream-fifth.json` and the raw
log. The graph helper still routes through C under `SYSTEM_TIG=1`; these
numbers are not a Rust completion percentage. The `grep` view, other view
actions, and broader compatibility remain open, so the end-to-end benchmark
is still gated.

## Sixth checkpoint verification

The Rust grep view now handles file headers, line hits, configured width,
interactive `g` queries, refspec blobs, and edit/command context for paths
that map safely to the worktree. Direct Git argv and NUL fields preserve
filenames containing colons and newlines. Revision-tree expressions, including
`HEAD:subdir`, resolve to their own tree for blob navigation. When a nested
tree cannot map back to a worktree-root path, editing and file-context commands
are disabled; ambiguous optioned ref paths and output-changing grep options
fail explicitly. `line-graphics=auto` now honors locale-variable precedence,
including non-UTF-8 values.

The four unchanged original grep scripts pass all 14 assertions. The original
width script passes 53 of 54 assertions; its remaining failure is the refs
`maxwidth` case. Formatting, all 45 Rust unit tests, Clippy with warnings
denied, release build, and 108 real PTY checks pass. An independent path-safety
review found no remaining concrete high-priority wrong-file route in this grep
slice.

The sixth full upstream run attempted all 154 recipes and reported **263 of
594 assertions failed**, in 150 tests with 3 skips and 1 recipe without a
result receipt. See `migration/evidence/rust-upstream-sixth.json` and its raw
log. The graph helper still routes through C under `SYSTEM_TIG=1`; early
exits alter assertion totals. These counts are not a migration-completion
percentage. Full parity and the end-to-end benchmark remain gated.

## Seventh focused checkpoint

The refs view now applies `maxwidth` to its inferred reference column while
respecting explicit `width` and percentage limits. The unchanged original
`test/tigrc/width-test` passes all 54 assertions on the Rust release binary;
Rust formatting, unit tests and Clippy also pass. This focused check does not
replace the sixth full-suite receipt; see
`migration/evidence/refs-width-seventh.json`. The parity gate remains open.

## Eighth checkpoint verification

Pager-family line-number columns, diffstat jumps and selected-file titles now
match the focused original editor cases. The help view is generated from active
bindings and supports the original section-collapse and search interactions.
Main-view `:view-diff` on working changes opens the worktree patch, including
unmerged conflict output, and refresh keeps Git's configured prefix. The five
unchanged original diff/status editor scripts pass **55 of 55 assertions**;
the two original help scripts and user-command script also pass, while the
config parse/source scripts now fail only on diagnostics, not help screens.

A safety review found two wrong-file staging routes while enabling conflict
diffs. Cached patch application now accepts only matching, repository-relative
canonical `a/` and `b/` paths; configured no-prefix patches fail closed before
`git apply`. Stage updates also reject a combined conflict patch even if an
ordinary file patch follows it. Real-repository and mixed-patch regressions
cover both cases. The independent P1 review found no remaining wrong-file
route in this slice. This intentionally leaves staging from a no-prefix patch
unsupported until path-stripping semantics can be implemented and verified.

Formatting, all **51 Rust unit tests**, Clippy with warnings denied, release
build and **108 real PTY checks** pass. The final binary SHA-256 is recorded in
`migration/evidence/rust-upstream-eighth.json`. The unchanged full upstream
suite attempted all 154 recipes and reported **220 of 593 assertions failed**
in 150 tests, 3 skips and 1 recipe without a result receipt. The raw log is
`migration/evidence/rust-upstream-eighth.log`. `SYSTEM_TIG=1` still routes the
graph helper through C, and early exits change assertion denominators; these
numbers are not a Rust completion percentage. Full parity and the application
benchmark remain gated.

## Ninth checkpoint verification

Unmerged status entries now appear only under unstaged changes and use Tig's
`U` marker, including `AA` and `DD` conflicts. A missing or empty
`rebase-apply/head-name` falls back to the current branch while other I/O
errors remain visible. Silent external commands can return a nonzero child
status without stopping a script or interactive refresh; launch failures still
fail. Echo takes precedence when both flags are present. The two unchanged
original branch-status scripts pass **36 of 36 assertions**.

The final binary passed formatting, **53 Rust unit tests**, Clippy with
warnings denied, release build, and **108 real PTY checks**. The full
upstream suite attempted all 154 recipes and reported **207 of 590
assertions failed** in 150 tests, with 3 skips and 1 recipe without a result
receipt. See `migration/evidence/rust-upstream-ninth.json` and its raw log for
the exact binary hash and failures. The graph helper still routes through C
under `SYSTEM_TIG=1`; early exits change assertion totals, so this is not a
completion percentage. Full parity and the application benchmark remain gated.

## Tenth checkpoint verification

Reviewed [diff-context/word-diff PR #1](https://github.com/naikokodayo/tig/pull/1)
is merged. It passes the two unchanged original context scripts (20 of 20
assertions), including diff refresh, returning to main, and a CLI
`--end-of-options` regression. The original harness ignores whitespace in
screen comparisons; a few whitespace-only rows still differ and this focused
result is not byte-for-byte screen parity.

After merge, formatting, **57 Rust unit tests**, Clippy with warnings denied,
release build and **108 PTY checks** pass. The full original suite attempted
all 154 recipes and reported **186 FAIL records among 589 OK/FAIL records** in 150 tests,
with 3 skips and 1 missing receipt. See
`migration/evidence/rust-upstream-tenth.json` and its raw log. This is still a
mixed helper route and early exits change the denominator, so the parity
gate remains open and the end-to-end application benchmark remains gated.
The FAIL total also includes process errors and timeouts, not only behavioral
assertion mismatches.

## Eleventh checkpoint verification

Reviewed [config parity PR #2](https://github.com/naikokodayo/tig/pull/2)
is merged. Formatting, 61 Rust unit tests, Clippy with warnings denied,
release build, and 108 PTY checks pass. The unchanged full original suite
attempted all 154 recipes and reported **184 FAIL records among 589 OK/FAIL
records** in 150 tests, with 3 skips and 1 missing receipt. See
`migration/evidence/rust-upstream-eleventh.json` and its raw log. This run was
on `3e8f4b8e`, before the subsequently merged blame PR #4. The FAIL total
includes process errors and timeouts, so it is not solely a count of
behavioral assertion mismatches. The graph helper still routes through C;
Rust full parity and the application benchmark remain gated.

## Rust-only original-test adapter

Use `python3 rust/tests/upstream-suite.py` for future original-suite evidence.
It runs C first and then explicitly selects **both Rust executables**, records
per-script exits and original assertions, and maps assertions not reached by
Rust. `make RUST_ONLY=1 test` provides the same application/helper routing;
`--self-test` on the Python runner verifies intentional failures cannot pass.
This replaces the mixed-helper adapter as the evidence method. The historical
ninth, tenth and eleventh mixed-helper receipts remain unchanged.

See [the versioned harness report](migration/UPSTREAM-HARNESS.md). Its full
Rust-only snapshot is based on `47f1a2b`; subsequent configuration and blame
merges have separate focused receipts, not a relabeled full-suite count.
The parity gate remains blocked, including missing `TIG_TRACE` semantics.
The final documentation-only synchronization to `34684f79` changes none of
the code validated after PR #4.


## Date compatibility slice (2026-09-27)

Raw `--pretty=raw` stdin now enters main, retains author/committer timestamps,
and redraws the owned input on date toggles. Previously it entered pager and
stopped the original date script: **9 failures in 9 assertions**, including
an unexpected exit. On this slice the unchanged date script passes **8/8**
(the extra exit failure disappears). Six original focused scripts pass **27/27**
on both the freshly built C reference and Rust. See
[`date-focused.json`](migration/evidence/date-focused.json) and its C/Rust logs.

Chrono **0.4.45**, with defaults disabled and only `std`, replaces handwritten
calendar validation and timestamp conversion. The added lockfile closure is
`num-traits 0.2.19` and build dependency `autocfg 1.5.1`; all support Rust 1.81
and offer a GPL-compatible MIT license option. See the
[dependency and semantic investigation](migration/chrono-date-compatibility.md).
Tig's relative thresholds and wording remain explicit. Nonlocal `%Z` emits the
commit's numeric offset; local/locale formatting uses GNU/BSD `date` with the
system timezone, DST rules and locale. `TEST_TIME_NOW` is shared with synthetic
change rows; invalid overrides return errors rather than falling back to now.

Rust 1.81 fmt, **55 unit tests**, Clippy with warnings denied, release build,
**108 PTY checks**, and **13 isolated date checks** pass on macOS arm64. Date
checks include a DST boundary, POSIX TZ, French locale, future and negative
timestamps, and error handling. The new script is also wired into Linux/macOS
CI; this local receipt does not assert those remote results. See
[`date-checks.log`](migration/evidence/date-checks.log) and
[`date-terminal-smoke.json`](migration/evidence/date-terminal-smoke.json).

Scope remains bounded: the full upstream suite was not rerun, and the historical
207/590 failure count must not be reduced arithmetically. Native formatting costs
one process per local/locale date and requires GNU/BSD `date`; UTF-8 output is
required. Format modifiers (`%E`, `%O`, padding flags), arbitrary libc extensions,
and C's pathological empty/escaped timezone-format behavior are not claimed:
unrecognized formats return explicit errors. Raw input supports ordinary Git
headers, not a complete reflog/boundary/decorations parser. First-party unsafe
code remains forbidden; original C files and tests are unchanged.

### Date slice sync with PR #1

Merged fork main `47f1a2b2` into the date branch without textual conflicts.
Integration review found that raw-stdin detection also needed to honor the new
`--end-of-options` boundary. A binary regression first reproduced raw text
incorrectly opening main; the detector now stops at either option terminator.

After that fix, Rust 1.81 fmt, **59 unit tests**, Clippy with warnings denied,
release build, **15 isolated date/argument checks**, and **108 PTY checks** pass.
The six original date-related scripts still pass **27/27**; the two original
PR #1 diff-context scripts additionally pass **20/20**, for **47/47** focused
assertions and zero failures. See [`date-sync.json`](migration/evidence/date-sync.json)
and its checks/focused logs for the new binary hash. Earlier date receipts
remain historical. The full upstream suite was not rerun.

### Date slice sync with PR #2 and zero-date review fix

Merged fork main `3e8f4b8e`; the only text conflict combined both appended
migration records. Configuration parsing/diagnostics and date toggles retain
both slices' behavior. The review's Unix-zero display regression first failed,
then passed with a display-only guard matching C `time->sec == 0`. This field
is commit wall time, so raw `0 +0000` and `-32400 +0900` display no date, while
`0 +0900` remains `1970-01-01 09:00 +0900`. Parsing still accepts valid Unix zero.
Five equivalent C binary probes confirm these display cases.

Rust 1.81 fmt, **64 unit tests**, Clippy with warnings denied, release build,
**20 isolated date/argument checks**, and **108 PTY checks** pass. Twelve original
focused scripts pass **69/69** assertions (main/date-related 27, diff-context 20,
configuration 22). The additional original quote-test remains **1 pass / 6 failures**;
its receipt exactly matches PR #2's saved record. This is **70 passes / 6 failures**
across all 13 scripts actually run, not an all-green suite. The full upstream
suite was not rerun. See [`date-config-sync.json`](migration/evidence/date-config-sync.json)
and its focused/check logs for the final binary and exact scope.

Performance limitation: every local/locale date still starts one system `date`
process, including repeated redraws. Large histories may therefore block the UI;
there is no cache, batching, or large-history performance claim. The zero-date
guard avoids a process for the sentinel, but is a compatibility fix, not a general
performance optimization. Measure representative histories before adding a
bounded cache or batching while preserving TZ/locale semantics.

### Date slice sync with PR #4 and eleventh evidence

Merged blame main `a28f69d` and then documentation main `34684f79`, preserving
pushed history. The parser insertion conflict retains both raw history and Git
path decoding. Blame now reuses the Chrono timestamp/offset converter instead
of a second handwritten Gregorian conversion; its existing 0..9999 year limit
is retained. Its rendering regression also checks blank epoch-zero dates. New
range tests first failed, then passed with an explicit checked offset conversion
at Chrono's minimum/maximum timestamp, preventing out-of-range local dates.

Rust 1.81 fmt, **67 unit tests**, Clippy with warnings denied, release build,
**20 date/argument checks**, and **108 PTY checks** pass. The final documentation
merge changes no tested Rust/Cargo files. Across 20 unchanged original recipes,
there are **73 OK / 22 FAIL records**: twelve main/diff/config scripts pass
**69/69**, seven blame scripts remain **3 OK / 16 FAIL** as reported by PR #4,
and quote-test remains **1 OK / 6 FAIL** with the exact PR #2 receipt. FAIL records
include process errors, not only screen mismatches. This is focused evidence,
not a full-suite rerun. See [`date-blame-sync.json`](migration/evidence/date-blame-sync.json)
and its focused/check logs. The per-local/locale-value subprocess limitation
also applies to blame lines; no performance improvement is claimed.


### Date slice: nonlocal `%s` gate and PR #5 harness sync

Merged main `03f6a29f` without rewriting pushed history; the additive migration
record conflict preserves both slices. Review reproduced a silent discrepancy:
C's nonlocal `%s` feeds wall time through libc `mktime` under the user's TZ with
`tm_isdst=0`, whereas the Rust backend had formatted it under UTC. The shared
formatter now explicitly rejects nonlocal `%s`; local `%s`, literal `%%s`, and
zero-date blank display remain supported. **Exact nonlocal `%s` compatibility
is an OPEN gate**, not a completed parity fix. No unsafe FFI or new dependency
was introduced. Primary-source rationale and follow-up boundary are recorded in
[`chrono-date-compatibility.md`](migration/chrono-date-compatibility.md).

At `f22e363`, Rust 1.81 fmt, **67 unit tests**, Clippy with warnings denied,
release build, **29 isolated date/argument checks**, and **108 PTY checks** pass.
The new upstream adapter runs six unchanged main/date scripts against C and
Rust-only binaries: **27/27 actual assertions pass on each side**, with no
missing assertions or runtime failures. The adapter's negative self-checks
also pass. See [`date-percent-s-checks.json`](migration/evidence/date-percent-s-checks.json),
[`date-percent-s-upstream.json`](migration/evidence/date-percent-s-upstream.json),
[`date-percent-s-pty.json`](migration/evidence/date-percent-s-pty.json), and
[`date-percent-s-harness-selftest.json`](migration/evidence/date-percent-s-harness-selftest.json).
The full suite was not rerun; earlier broader receipts remain historical.
Original C sources and original tests are unchanged by this date slice.

### Full strict C/Rust paired snapshot after date integration

The strict runner attempted all 154 unchanged original scripts against both
C and Rust-only application/graph binaries at source `e1cf5eeb96166b4971df2f84a2c93127edbd1549`.
C passed 152 scripts, failed none, and skipped two. Rust passed 82, failed 69,
and skipped three. Rust reached 416 passing and 139 failing real assertions;
17 assertions reached by C were not reached by Rust. Another 34 failure
records concern setup or runtime behavior and are not assertion failures.
The [paired receipt](https://github.com/naikokodayo/tig/blob/2fb2a87105b77b0c2de37d6be766365acaa5ac1d/migration/evidence/upstream-rust-only-after-date.json)
includes binary hashes, raw transcripts, and per-script outcomes. The parity
gate remains **OPEN**. This source snapshot predates the later stage,
save-options, and refs merges; these numbers are not a result for current main.

## Diff input, pane width and navigation slice (2026-09-27)

This slice starts from fork main `2fb2a871` in an independent clone. No
`AGENTS.md` is present in that checkout. The old after-date full-suite snapshot
remains historical; a fresh strict paired run reproduces all ten requested
scripts failing on Rust, while C passes all 23 assertions. See
[`diff-render-before.json`](migration/evidence/diff-render-before.json).

Three bounded mechanisms are corrected:

- `tig show` consumes supplied diff text, retains its commit ID and redraws it
  without resolving HEAD or re-reading Git. Empty input and terminal control
  characters are handled safely. Forwarding revision lists with `show --stdin`
  remains explicitly unsupported, rather than displaying revision names as a patch.
- Vertical splits reserve the separator in the child pane, matching C's 91/89
  content widths at 181 columns. Initial diff loads and diff/log refreshes pass
  the actual pane width to Git's native stat formatter. Script dimensions are
  applied before loading content. Log width does not enable stats when disabled.
- Explicitly opening the current diff detaches its parent navigation, including
  after maximize. `next` then moves within that diff instead of opening the
  parent's next commit. The parent remains available through view-close.

At source `3b028e0cdedf34e9edd242fd28937f5d5d18464c`, the ten requested scripts
now have **4 passing / 6 failing scripts**, with **15 OK / 8 FAIL assertion
records and 1 additional runtime failure**. C passes all ten and all 23
assertions. The newly passing scripts are diff-stat-split, diff-stdin,
maximized-navigation and open-after-split. The log diff-stat refresh assertion
also passes; its initial split assertion still fails.

The final paired run adds twelve related, unchanged diff/editor/log/main/width
scripts: **C 22/22 scripts and 160/160 assertions; Rust 16 passing / 6 failing
scripts, 152 OK / 8 FAIL assertion records and 1 additional runtime failure**.
No scripts are skipped and the adapter reports no unmatched assertion IDs.
The missing `view.data` output is counted as a failed assertion, not a pass.
This is focused evidence, not a full-suite result or a completion percentage.
See [`diff-render-after.json`](migration/evidence/diff-render-after.json) for
source, executable routing/hashes, original script hashes and raw transcripts.

Remaining requested failures:

- `diff/commit-title-wrap-test` and `diff/wrap-lines-test`: visual wrapping and
  continuation markers are still missing. Supplied title text now loads.
- `diff/diff-highlight-test`: configured external highlighter output is not used.
- `diff/diff-stat-test`: `save-view` and its typed cell dump are unimplemented;
  the script exits and its expected output is absent.
- `diff/line-number-test`: the command output is not opened as a pager view.
- `log/diff-stat-test`: the initially loaded, wide parent stat rows need cell-aware
  truncation after splitting; refreshing the parent regenerates matching rows.

Rust 1.81 formatting, **70 unit tests**, Clippy with warnings denied, release
build, **130 real PTY checks**, **6 supplied-diff input checks**, and **29 existing
date/argument checks** pass. [`diff-render-checks.json`](migration/evidence/diff-render-checks.json)
binds the source manifest and release binary SHA-256 to the
[check log](migration/evidence/diff-render-checks.log); the separate
[PTY receipt](migration/evidence/diff-render-pty.json) carries the same source
and binary identity. Reproduce the added boundary checks with
`python3 rust/tests/diff-input.py` after the release build.

First-party Rust remains unsafe-free. No dependency, original C source or
original test was changed. Full parity and application benchmarks remain gated.

### Full strict paired snapshot after stage, save-options, and refs

At source `2fb2a87105b77b0c2de37d6be766365acaa5ac1d`, the same fail-closed
runner again attempted all 154 unchanged scripts with separately hashed C and
Rust-only application/graph binaries. C passed 152 and skipped two. Rust passed
91, failed 60, and skipped three; 452 actual assertions passed, 105 failed, and
15 C-side assertions were not reached. Another 25 failure records concern
setup/runtime behavior. Compared with the earlier date snapshot, nine scripts
became passing: two refs, four stage, and three tigrc. The full [paired
receipt](https://github.com/naikokodayo/tig/blob/9a691ee1e0a940a9fee90158f93d81ec361b7ac3/migration/evidence/upstream-rust-only-after-stage-save-refs.json)
records every transcript and reason. This predates the merged main-graph PR #8
and subsequent work, so it is not a current-main or final benchmark result.

### Blame navigation and origin tracing (2026-09-27)

The blame view now uses Git's original line, historical filename and `previous`
commit/path metadata when reopening a blamed version or its parent. Parent
selection follows the zero-context blob diff, two-dot revision ranges retain
their lower bound, and back restores the saved arguments and position. Enter
opens the blamed file's diff at the corresponding line; toggling file filtering
keeps that line in view. Deleted diff lines trace the old path/line before
opening their origin, including stash diffs. Configured horizontal scrolling
and numeric commands with trailing annotations are honored.

The initial receipt started at `2fb2a871` and synchronized `9a691ee1`; its tested
code is `1ec4145de69084e099720a6c28dadf36392ad65b`. The
[before receipt](migration/evidence/blame-navigation-before.json) records all six
requested scripts passing under C and failing under Rust (2 passing assertions,
11 failing assertions and 5 additional runtime failure records). The
[after receipt](migration/evidence/blame-navigation-after.json) records the exact
source, C/Rust binary hashes, unchanged script hashes and per-assertion mapping.
The requested six scripts now reach **12/13 passing Rust assertions**, without
runtime failures; C passes **13/13**. The remaining initial-diff assertion is
C's curses `x` separator versus Rust's `│`; origin path, line and viewport agree.
This is still a failing original assertion, not a parity pass.

Across all 15 original scripts actually run, C passes **89/89**, while Rust
passes **87/89**. The other mismatch is the existing stash-list column/title
format in `test/stash/start-on-line-test`; stash-to-diff-to-blame passes. The
[check receipt](migration/evidence/blame-navigation-checks.json) and linked logs
record Rust 1.81 formatting, 72 unit tests, Clippy, release build, 130 existing PTY
checks and six added rename/boundary/navigation checks. Old-side path decoding
also has a regression preventing an unprefixed `a/file` from resolving to
`file`. No dependency, original C source or original test was changed.

Scope remains limited: combined-diff blame tracing and the broader blame option
surface (including copy-following flags) remain unsupported; full stash rendering,
nested view behavior and curses screenshot encoding remain open migration work.
The complete original suite was not rerun for this slice, and full parity and
end-to-end benchmarks remain gated.

### Full strict paired snapshot after main graph, date `%s`, and prompt fixes

At source `c097ffb8a03f34f91accb9287ea299313f316c7e`, the fail-closed
runner again attempted all 154 unchanged original scripts with separately
hashed C and Rust-only application/graph binaries. C passed 152 and skipped
two. Rust passed 95, failed 56, and skipped three; 462 actual assertions
passed, 95 failed, and 15 C-side assertions were not reached. Another 21
failure checks concern setup or runtime behavior rather than assertions.
The [paired receipt](https://github.com/naikokodayo/tig/blob/b0eabd2cf503ec5f220dd1d160ebe47866563a89/migration/evidence/upstream-rust-only-after-main-date-prompt.json)
records every transcript and reason. This source predates merged PR #12, so
these numbers are not current-main or final benchmark results. The parity gate
remains **OPEN**.

### Full strict paired snapshot after diff, refs, and tree integration

At source `86ce768063fa1785eb224421c0edc56e31d351ea`, the fail-closed
runner attempted all 154 unchanged original scripts with separately hashed C
and Rust-only application/graph binaries. C passed 152 and skipped two. Rust
passed 104, failed 47, and skipped three; 475 actual assertions passed, 82
failed, and 15 C-side assertions were not reached. Another 19 failure checks
concern setup or runtime behavior rather than assertions. This is nine more
passing Rust scripts than the preceding `c097ffb8` snapshot. The [paired
receipt](https://github.com/naikokodayo/tig/blob/ac78df21291964083a6cb13a08da5d127fdcc60e/migration/evidence/upstream-rust-only-after-refs-tree-diff.json)
records every transcript and reason. This source predates merged status/trace
and config PRs #13 and #16; it is not a current-main or final benchmark result.
The parity gate remains **OPEN**.

### Full strict paired snapshot after navigation and configuration integration

At source `ac78df21291964083a6cb13a08da5d127fdcc60e`, the fail-closed
runner attempted all 154 unchanged original scripts with separately hashed C
and Rust-only application/graph binaries. C passed 152 and skipped two. Rust
passed 121, failed 30, and skipped three; 517 actual assertions passed, 47
failed, and eight C-side assertions were not reached. Another 11 failure checks
concern setup or runtime behavior rather than assertions. The [paired
receipt](https://github.com/naikokodayo/tig/blob/72133541191c0b3d3e458fcb6075637a9ffe36ef/migration/evidence/upstream-rust-only-after-integration.json) contains
the executable hashes and individual transcripts. This is 17 more passing
Rust scripts than the preceding `86ce7680` snapshot. The parity gate remains
**OPEN**; completed follow-up slices must be retested against their merged source.

### Current strict paired snapshot after diff, stdin, Git alias, and blame integration

At merged source `d59e2d52f719ef5b43ff501cadd8852119f2a61e`, the runner
attempted all 154 unchanged scripts with separately hashed C and Rust-only
application/graph binaries. C passed 152, failed none, and skipped two. Rust
passed 141, failed 10, and skipped three: 554 actual assertions passed, 16
failed, and two C assertions were not reached. The [current paired
receipt](migration/evidence/upstream-rust-only-current.json) contains each
script's original output, route and hashes. This is seven more passing Rust
scripts than the [preceding source-bound snapshot](https://github.com/naikokodayo/tig/blob/d59e2d52f719ef5b43ff501cadd8852119f2a61e/migration/evidence/upstream-rust-only-current.json).
`tree/file-name` exposes C's quoted-text directory stripping bug; the Rust
path remains byte-preserving and its original assertion is still recorded as
failed. `stage/split-chunk` is a display mismatch, not evidence of C index
data loss in this script. The parity gate remains **BLOCKED**;
the end-to-end C/Rust benchmark has not begun.

### Non-local `%s` follow-up

The PR #3 blanket refusal is replaced by a host POSIX bridge on 64-bit macOS
and GNU/Linux with 64-bit system Perl and its core POSIX module. First-party
unsafe remains forbidden; no Cargo dependency is added. Missing tools, other
platforms and invalid output fail explicitly. The implementation retains the
commit wall time and libc's `tm_isdst=0` behavior, including summer dates.
Runtime requirements, evaluated Rust alternatives, differential/upstream
receipts and the still-open performance/full-migration boundaries are in the
[updated date compatibility record](migration/chrono-date-compatibility.md#非本地-s系统-posix-桥接).

### Configuration compatibility diagnostics and shared column truncation

This slice starts at fork main `15ee9762`, in an independent clone. The full
strict `upstream-rust-only-after-stage-save-refs.json` remains historical at
`2fb2a871`; it is not relabelled as current-main evidence. PR #12's diff-input,
stat-width and navigation changes are not included or duplicated here.

The config parser now diagnoses removed options, legacy key notation, renamed
bindings/colors and old date modes with upstream wording. Supported replacements
are retained before issuing their warnings; obsolete options remain rejected.
Unknown view-column diagnostics use the original value. Field trimming is shared
by grep, main, tree and blame: `utf8`/`utf-8` renders `⋯`, one-cell literal values
are honored, and empty/wide/multi-cell delimiters fall back to `~`. Grep filename
clipping/padding now measures terminal cells, including wide/combining characters.
This covers displayed fields, not save-options normalization of delimiter spelling
or implementation of terminal color attributes.

Validation at source `5e9f51a37dd7e376681914bfd231767a1c0c3f4b` on macOS arm64,
Rust 1.81 (the next commit adds only documentation/evidence):

- Formatting, all **74 unit tests**, Clippy with warnings denied, release build,
  and **130 real PTY checks** pass. New checks first reproduced the diagnostic
  and delimiter failures and now pass; original C sources/tests are unchanged.
  First-party unsafe remains forbidden, with no new dependency.
- Fresh paired baseline for the six requested original scripts: C passes all
  **26 assertions**; Rust fails all six scripts with **13 passing / 13 failing
  assertions**, plus **2 runtime failure checks**.
- Final requested scope: C still passes **26/26**; Rust passes three scripts,
  with **19 passing / 7 failing assertions**, plus the same **2 runtime checks**.
  Newly passing scripts are `compat-error-test`, `truncation-test`, and
  `view-column-test` (all **17/17 assertions** in this fixed slice).
- Expanded scope is all 17 tigrc scripts, all four grep scripts, and main/default,
  tree/default, tree/recurse, blame/default: C passes **25/25 scripts, 142/142
  assertions**; Rust passes **21 scripts**, with **132 passing / 10 failing
  assertions**, plus **3 runtime failure checks**. There are no skips.

The three requested failures still concern command-output paging
(`command-value-long-test`), unsupported selected `refname` expansion
(`escape-var-test`), and prompt-variable/quoted command handling (`quote-test`).
The expanded blame/default test also fails its diff/navigation cases. These are
failures, including missing output, not skipped assertions. The full suite was
not rerun and the migration parity gate remains **OPEN**.

The branch-phase before/after transcripts, hashes and scoped checks are
archived in [commit `aaedd5b7`](https://github.com/naikokodayo/tig/tree/aaedd5b7890dba99fad72752a366a83e1cf773a8/migration/evidence).
The paired runner exits 1
with `BLOCKED`, as required by the remaining failures. Reproduce the expanded
pair with `python3 rust/tests/upstream-suite.py test/tigrc/*-test test/grep/*-test
test/main/default-test test/tree/default-test test/tree/recurse-test
test/blame/default-test` (one shell command).
### Tree startup directories and editor paths

At source `bf5d0d85ce159bdb38f7bb4dd416e0d021bd162b`, first opening the tree
uses the canonical invocation directory relative to Git's worktree root. It
applies that prefix only once, including after closing and reopening the view;
a failed load does not consume initialization. Bare repositories use an empty
prefix. Repository discovery continues to distinguish a submodule's or linked
worktree's root from its separate Git directory. Editor arguments retain the
repository-relative filename and execute at that worktree root, not in the
superproject or Git metadata directory.

The strict paired runner passes **32/32 C assertions** and **31/32 Rust
assertions** across all six original tree scripts. Both submodule-editor and
worktree-editor now pass all six assertions, including screens, editor content,
working directory, Git directory and superproject context. Before this change,
the three targeted scripts passed 13/16 Rust assertions at main `15ee9762`;
this reproduction is separate from the older `2fb2a87` full-suite snapshot.

**Remaining difference:** `test/tree/file-name-test` still fails
`first-child-dir.screen`. C `tree_read` strips the directory's byte length from
Git's quoted filename before decoding it, displaying a truncated octal-escaped
name without history metadata. Rust's NUL-delimited parser retains `as测试asd`
and its metadata. This patch does not imitate that corrupted display/path or
change the original assertion. A regression verifies the real Unicode blob and
editor argument, including the leading-dash directory's `./` editor protection.
The full parity gate remains **OPEN**.

Rust 1.81 formatting, **72 Rust tests**, Clippy with warnings denied and **135
PTY checks** pass, including a new interactive startup-directory/parent case.
Original C sources, headers and tests are unchanged; first-party unsafe remains
forbidden. The [receipt](migration/evidence/tree-paths/receipt.json) binds source
and binary hashes to the before/after paired runs, failing-then-passing regression,
and PTY evidence. No full upstream suite or end-to-end benchmark was rerun.


### Refs filtering and replacement follow-up

The unchanged `test/refs/filter-test` and `test/refs/replace-test` first reproduced
four failed Rust assertions while C passed all four, matching the historical
`2fb2a871` strict snapshot. `Repository::refs` now executes `TIG_LS_REMOTE` as
explicit argv using the existing config tokenizer; a shell runs only when the
configured program itself is a shell. Nonzero commands and malformed output fail
explicitly. Main history uses the same reference source and existing numeric/type
ordering as refs, including tracking-remote priority. Replacement-only records
remain main decorations, named replaced branches keep their name, and the refs
list omits anonymous replacement rows. Filtered-out HEAD is not synthesized back
into the refs view; ordinary detached HEAD still has a shared reference record.

At source `9cb10a19`, formatting, **72 Rust tests**, Clippy with warnings denied,
and **130 real PTY checks** pass. All nine original refs scripts plus main default,
main search and column width pass **90/90 actual assertions on each of C and Rust**,
with no missing assertions or runtime failures. The branch-phase before/after
transcripts and checks are archived in [commit `2c62bbf9`](https://github.com/naikokodayo/tig/tree/2c62bbf961196704c17adf236b1830239c2ca5ff/migration/evidence);
the final integrated receipt is linked below. The archived runner's commit field
is the clean starting base; its tested dirty sources are explicitly bound to
`9cb10a19` by that manifest.
C sources, original tests, dependencies, and `rust/main.rs` are unchanged.

This is a focused slice, not a full-suite rerun or byte-for-byte terminal parity
claim. Raw-stdin history decorations, all reference-format subclasses, duplicate
custom command records, annotated-tag/branch name collisions in main, and complex
replacement alias/chain behavior have not been established by this receipt.
Loading remains synchronous and uncached; this is not a performance claim.
The full migration gate remains open. PR #12/#13 main-view changes are outside
this diff; the custom command spawn will need the same trace integration as other
external commands when the separate trace slice is integrated.

### Diff review fix and main synchronization

The review found that Enter → maximize → next retained fullscreen presentation
but loaded the next diff with the child pane's stat width. The new paired
`rust/tests/diff-navigation.py` regression failed before the fix: Rust truncated
a long stat filename while C retained it. Child opening now receives the intended
split/fullscreen state before loading Git output; next/previous preserve that
state instead of restoring it only after loading. Initial split behavior remains
covered. At 180 and 181 columns, all eight comparisons (split, maximized next,
maximized previous, and refresh) now match C.

Merged main through `c097ffb8` (including PR #11) without rewriting published
history. The MIGRATION.md append conflict retains both prior records; automatic
code merges retain the history-graph/date changes and prompt/view-close fixes.
All earlier diff receipts remain historical and unchanged.

Final source `000e985da7814b55e652e19d50d0ee8a10c1146c` passes Rust 1.81 fmt,
**72 unit tests**, Clippy, release build, **130 PTY checks**, **6 stdin checks**,
**38 date/argument checks**, and the **8 new paired navigation comparisons**.
The final original-test scope is the previous 22 scripts plus the two PR #11
prompt/script regressions: **C 24/24 scripts and 169 OK assertions; Rust 18
passing / 6 failing scripts, 161 OK / 8 failed assertion records and one extra
runtime failure**. The same six previously documented failures remain; the new
navigation regression and both PR #11 scripts pass. No full-suite claim is made.

Final source manifests, binary hashes and logs are retained in
[`diff-render-review-checks.json`](migration/evidence/diff-render-review-checks.json),
[check log](migration/evidence/diff-render-review-checks.log),
[PTY receipt](migration/evidence/diff-render-review-pty.json),
[paired navigation receipt](migration/evidence/diff-render-review-navigation.json),
and [original-script receipt](migration/evidence/diff-render-review-upstream.json).
Reproduce the new paired check with `python3 rust/tests/diff-navigation.py`
after building both C and Rust binaries. Original C/tests and dependencies are
unchanged relative to the integrated main; first-party Rust still forbids unsafe.

### Main navigation and refresh slice (2026-09-27)

Main's `parent` and `<`/`back` now preserve row, vertical offset and horizontal
offset through refresh and staging. Empty history stays in main; a root/missing
parent and out-of-range numeric jump leave the selection in place. `--merge`
requests boundary commits, whose markers now reach both graph renderers.
Diff headers append Git's describe result when the commit has no direct tag.

Prompt `:!command` reuses safe argv expansion and opens a maximized command-output
pager. The prompt, scripted command and interactive command paths share
`refresh_after_command`; their previous inline refresh calls are replaced.
It refreshes both displayed panes or the cached main/status parent of a
full-screen view. Closing a split's command pager returns to main without
restoring a stale diff. Bound external commands and `:exec` retain their
foreground/confirmation semantics.

Paired source `3ae580f8e3a9c4843f9112d130179d97e460777e` integrates main
`0d80d985`, including trace/status, tree, refs, configuration and blame changes.
The earlier documentation conflict retained both records; the final merge
needed no manual code resolution. Original C/tests, dependencies and Actions
are unchanged by this slice; first-party Rust still forbids unsafe.

The [single final receipt](migration/evidence/main-navigation-final.json)
contains source/binary SHA-256 manifests, original assertion results and raw
failure output, plus necessary safety regressions. Intermediate before/after
receipts remain available in Git at `83c06b65`; no duplicate final receipt
sets are retained. The four requested scripts improved from **14/28** Rust
assertions at `c097ffb8` to **27/28**: goto, jump-ends and refresh pass in full.
The final 20-script pairing passes **94/94 C assertions** and **92/94 Rust
assertions** (18 scripts pass, two fail), with no skips or missing assertions.

Rust 1.81 formatting, **83 unit tests**, Clippy with warnings denied, release
build, **150 PTY checks**, **6 diff-input checks**, **8 paired diff-navigation
comparisons**, **6 blame-navigation checks** and **12 blame review probes** pass.
The strict original-test gate remains **BLOCKED**: the narrow `view-split` date
column uses a separator cell, and `main-options` ignores the history limit.
Command-output capture still places stderr after stdout and uses the existing
synchronous loader. No full-suite rerun or completed-migration claim is made.

Review fix `f31cc142` refreshes cached parents using their own args/revision/path,
then restores the active context even on error. The real-repository regression
`python3 rust/tests/main-command-refresh.py` failed with ambiguous `needle`
before the fix; both fullscreen grep exec/pager commands now pass and return
to main correctly. The existing main navigation and staging tests, formatting,
Clippy and release build pass. The same receipt contains this small source/hash
and red/green supplement; the earlier broad results retain their original source.

### Configuration slice review fixes and main 297e1787 synchronization

Merged main `297e1787` (including PR #12); the only conflict was the appended
migration record, resolved by retaining both records. Source
`0f1835c2a174051409789ffa3c15db0b3296bba8` fixes three review findings:

- Setting an existing date column from `custom` to an invalid/removed display
  now installs `default` before returning C's diagnostic. Both `*-view-date`
  and `*-view-date-display` retain other column attributes. Whole-view
  replacement still discards failed new columns, as C does. Tests cover file
  and interactive entry points, local/short, uppercase and unknown modes.
- Field width measurement now counts the same Unicode scalars as clipping.
  ZWJ emoji cannot be partially clipped without the truncation marker because
  of a grapheme-width/scalar-width mismatch. Main, tree, blame and grep use
  this measurement for field limits and padding.
- Legacy color lookup separates the explicit view prefix before mapping the
  area, preserving that prefix over the replacement's default view.
  `tree.tree-head` maps to `tree.header`, `diff.tree-head` to `diff.header`,
  and `main.main-revgraph` is rejected with its obsolete diagnostic.

The three regression assertions first failed on the previous implementation.
The new runnable `python3 rust/tests/config-recovery.py` also compares C/Rust
screens, diagnostics and saved color targets in **16 passing PTY cases**.
At the source above, Rust 1.81 formatting, **76 unit tests**, Clippy with
warnings denied, release build and **130 application PTY checks** all pass.
The review-phase checks, recovery comparison and PTY receipts are archived in
[commit `a11e32b5`](https://github.com/naikokodayo/tig/tree/a11e32b5/migration/evidence).
Their source manifest and binary hashes agreed with the paired original run.

The original-script scope is the previous 25 plus main/date and main/emoji:
C passes **27 scripts / 156 assertions**. Rust passes **23 scripts** with
**146 passing / 10 failing assertions** and **3 runtime failure checks**;
there are no skips. The same four scripts fail: command-value-long,
escape-var, quote, and blame/default. All three originally fixed tigrc
scripts still pass. The archived scoped raw result remains `BLOCKED`; these
results do not close full parity.
Earlier receipts remain tied to their earlier sources. The following commit
changes only documentation and evidence. First-party unsafe remains forbidden;
original C/test files and dependencies are unchanged.
### Tree review fix: explicit worktree outside the invocation directory

Review reproduced `prefix not found` when `GIT_DIR` and `GIT_WORK_TREE` point
to a valid repository but the process cwd is outside its worktree. The previous
filesystem-prefix assumption is replaced with `git rev-parse --show-prefix`
executed in the discovery directory. Git supplies the empty prefix for this
case. Nonempty paths still require repository-relative normal components;
component collection removes Git's trailing separator without decoding filename
bytes. The discovery directory is private again. First-open/failed-load behavior
is retained.

Merged main `297e1787` (PR #12) into the published branch. The open-view conflict
preserves main's requested rendering width and this branch's directory
initialization; both migration records remain intact. At integrated source
`be9ebd86ba16238dd66ff709e8d25b2e0e9dc9eb`, fmt, **73 Rust tests**, Clippy with
warnings denied, **139 PTY checks**, **8 paired diff navigation comparisons**
and **6 diff input checks** pass. The new real-environment PTY case failed on
the previous binary and passes now; a separate C/Rust scripted PTY comparison
produces identical root-tree screens from an external cwd.

The 11 unchanged original tree/diff scripts pass **99/99 C assertions** and
**98/99 Rust assertions**. The sole remaining difference is the previously
recorded Unicode filename screen; the parity gate remains OPEN. No full suite
or benchmark was run. Updated source/binary hashes, negative regression and
all focused evidence are in the [review-fix receipt](migration/evidence/tree-paths/review-fix/receipt.json).


### Refs follow-up sync after PR #12

Merged main `297e1787`; the only conflict was appended migration documentation,
resolved by retaining both records. No refs change was needed in `rust/main.rs`.
At merge source `e8fdc0f2`, fmt, **73 Rust tests**, Clippy, **130 PTY checks**,
**6 diff-input checks**, and **8 paired diff-navigation checks** pass. The earlier
12-script refs/main/width scope plus two diff-stat/navigation originals now passes
**96/96 assertions on both C and Rust across 14 scripts**. No missing assertions
or runtime failures occurred. The exact source/binary manifest and new receipts
are in [`refs-filter-replace-sync/checks.json`](migration/evidence/refs-filter-replace-sync/checks.json).
Earlier receipts remain unchanged; this does not close the full migration gate.


### Blame review and integrated-main validation (2026-09-27)

Integrated main `86ce7680` and tested source `736b925ec2a8146ac0f52e2afe6e506f972ddde9`.
The shared patch parser now removes literal tab header delimiters before decoding
paths, preserving spaces and genuinely quoted tab filenames. Blame tracing first
requires an ordinary hunk in the current file, then treats `--- deleted text` as
a deleted content line. Replacement-line Enter still selects `-old`, and tracing
returns to its older origin: paired execution confirmed that this is C's behavior,
and the user explicitly chose to preserve it.

[Review probes](migration/evidence/blame-review.json) cover ten paired C/Rust
checks and two Rust quoted-tab regressions. All twelve pass. C stays in diff for
quoted-tab filenames; those two Rust checks are not claimed as C parity.
The [integrated check receipt](migration/evidence/blame-navigation-sync-checks.json)
records formatting, **76 unit tests**, Clippy, both builds, **139 existing PTY
checks**, six blame rename/boundary checks, six diff-input checks, and eight paired
diff-navigation comparisons. Source, script and executable hashes are recorded.

The [fresh original-script receipt](migration/evidence/blame-navigation-sync-after.json)
contains 17 scripts: **C 98/98, Rust 96/98 assertions**,
with no runtime failures or timeouts. The six requested blame scripts remain
**C 13/13, Rust 12/13**. The only failures remain the saved `x`/`│` separator in
initial-diff and the existing stash-list columns/title. Earlier `1ec4145d` receipts
remain historical and unchanged. Original C, headers, original tests and dependencies
are unchanged; first-party unsafe code remains forbidden. Full parity remains gated.

### Configuration review fixes after refs main integration

After pushing the review fixes, main advanced to `b056a7ac` (PR #17). Merged it
without rewriting history; both appended documentation records are retained and
the refs changes in the shared renderer coexist with scalar field measurement.
At source `5ec2d9879f378b1eacf03647546196b3e3b05765`, formatting, **77 unit tests**,
Clippy, release build, **16 C/Rust recovery checks**, and **130 PTY checks** pass.
The expanded original pairing adds all nine refs scripts: C passes **36 scripts
and 178 assertions**; Rust passes **32 scripts**, with **168 passing / 10 failing
assertions** and **3 runtime failure checks**, no skips. The same four scripts
remain failed: command-value-long, escape-var, quote, and blame/default.

`migration/evidence/config-render-main-sync-{checks,recovery,pty,upstream}.json`
and `config-render-main-sync-checks.log` record this final integration's source
manifest and matching binary hashes. The preceding 297e1787 receipts remain
historical. This is still scoped validation with an open full-migration gate;
the next commit adds only this record and its evidence.

### Configuration review fixes after tree main integration

Main advanced again to `86ce7680` (PR #15). Source
`7979862af0570c965d2f9f6530ff4b23017c00e8` merges it, retaining both appended
migration records and both adjacent tree/grep unit tests. Formatting, **78 unit
tests**, Clippy, release build, **16 C/Rust recovery checks**, and **139 PTY checks**
pass. The targeted merge check pairs the six requested tigrc scripts and all six
tree scripts: C passes **12 scripts / 58 assertions**; Rust passes **8 scripts**,
with **50 passing / 8 failing assertions**, **2 runtime failure checks**, no skips.
The three command failures remain; tree/file-name retains the same first-child
Unicode filename snapshot mismatch recorded by PR #15's review-fix receipt.
The obsolete diagnostic and truncation scripts still pass, as do both tree
editor scripts. This is not a rerun of the earlier 36-script scope or full suite.

The tree-sync checks, recovery, PTY and original-script receipts are archived in
[commit `4a6e13fd`](https://github.com/naikokodayo/tig/tree/4a6e13fd33144b9ff559df839ad99c254a8614b0/migration/evidence).
The final main-sync receipts below remain in the current tree. The next commit is documentation and
evidence only. The three P2 fixes are ready for re-review; full parity stays open.


### Blame final main synchronization (2026-09-27)

Merged main `b951cd9e` once after PRs #13 and #16; tested source `90713600fee60a1a5a3448ce37505eafa590ccc7`.
The documentation conflict retained both records; no new behavior was added.
[Final checks](migration/evidence/blame-navigation-final-checks.json) record
formatting, **83 unit tests**, Clippy, release build, **140 existing PTY checks**,
six blame-navigation checks, twelve review probes, six diff-input checks and eight
paired diff-navigation comparisons. The twelve review probes retain their scope:
ten paired C/Rust checks and two Rust quoted-tab regressions.

[Final original-script pairing](migration/evidence/blame-navigation-final-after.json)
records **C 98/98, Rust 96/98 assertions across 17 scripts**,
with no runtime failure or timeout. The six requested blame scripts remain
**C 13/13, Rust 12/13**; the separator and stash-list format remain the only failures.
Earlier receipts are unchanged and remain bound to their own source commits.
Full migration parity remains gated.
