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
  Custom `TIG_LS_REMOTE` loading, exact reference sort ties and all mailmap/date
  configuration effects still need compatibility work.
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
The [paired receipt](migration/evidence/upstream-rust-only-after-date.json)
includes binary hashes, raw transcripts, and per-script outcomes. The parity
gate remains **OPEN**. This source snapshot predates the later stage,
save-options, and refs merges; these numbers are not a result for current main.

### Full strict paired snapshot after stage, save-options, and refs

At source `2fb2a87105b77b0c2de37d6be766365acaa5ac1d`, the same fail-closed
runner again attempted all 154 unchanged scripts with separately hashed C and
Rust-only application/graph binaries. C passed 152 and skipped two. Rust passed
91, failed 60, and skipped three; 452 actual assertions passed, 105 failed, and
15 C-side assertions were not reached. Another 25 failure records concern
setup/runtime behavior. Compared with the earlier date snapshot, nine scripts
became passing: two refs, four stage, and three tigrc. The full [paired
receipt](migration/evidence/upstream-rust-only-after-stage-save-refs.json)
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

This branch started at `2fb2a871` and synchronized `9a691ee1`; the final tested
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

### Non-local `%s` follow-up

The PR #3 blanket refusal is replaced by a host POSIX bridge on 64-bit macOS
and GNU/Linux with 64-bit system Perl and its core POSIX module. First-party
unsafe remains forbidden; no Cargo dependency is added. Missing tools, other
platforms and invalid output fail explicitly. The implementation retains the
commit wall time and libc's `tm_isdst=0` behavior, including summer dates.
Runtime requirements, evaluated Rust alternatives, differential/upstream
receipts and the still-open performance/full-migration boundaries are in the
[updated date compatibility record](migration/chrono-date-compatibility.md#非本地-s系统-posix-桥接).
