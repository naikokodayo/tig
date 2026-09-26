# Rust-only original-test evidence

The ninth checkpoint's `SYSTEM_TIG=1` result mixed the Rust application with
C's `test-graph`. Its assertion denominator also excluded scripts which
exited before writing `.test-result`. Neither count established full Rust
parity. The source base for this repair is `e94a87b43eba520d0c0e54d1dd19c3d6ddd367b6`.
There is no `test/Makefile` on that revision: the root `Makefile` owns routing.

## Reproduce

Use a dedicated checkout; do not run two original suites concurrently.
The original harness deletes its own `test/tmp/<script>` work directories.
POSIX (macOS/Linux), Python 3.9+, Git, make, a C compiler and Cargo are required.

```sh
python3 rust/tests/upstream-suite.py --self-test
python3 rust/tests/upstream-suite.py --output migration/evidence/upstream-rust-only.json
```

The default runner enumerates the tracked original `test/*-test` scripts.
It builds C `src/tig` plus `test/tools/test-graph`, runs all selected scripts,
then builds Rust `target/release/tig` plus `target/release/test-graph` and runs
the same scripts. It records both binary paths/SHA-256 values, build logs,
source/harness/script hashes, per-script transcripts and original receipts.
It verifies the selected executable routes and binary stability. This is
provenance for a trusted checkout/build, not protection against an adversary
replacing binaries during execution.

The environment removes inherited Git/Tig/test options, pins
`init.defaultBranch=master`, and lets the original harness establish its own
HOME, locale, terminal size and local Git configuration. Every script gets
a controlling 80×30 PTY and a 120-second outer deadline (configurable using
`--script-timeout`); the original per-invocation timeout remains intact.
The physical PTY size matters: Rust loads its first view before applying
scripted `COLUMNS`/`LINES`; a 0×0 PTY introduces unrelated display failures.
The outer deadline also covers graph helpers and setup, and kills the test
process group. Assertions and `libtest.sh` are unchanged.

For ordinary interactive runs, `make -k RUST_ONLY=1 test` uses the same Rust
binary directory for both commands, builds both bins together and fails if
either executable is missing. It rejects `SYSTEM_TIG=1` to prevent ambiguous
mixed routing. Use the Python runner for per-script evidence: the legacy
Make summary alone does not account for early exits without receipts.

## Verdict and mapping

Each script records its exit code separately from original `[OK]`/`[FAIL]`
checks. A nonzero script exit, an outer timeout, any failed check, or neither
a receipt nor an explicit skip causes failure. An exit-zero script with a
failed original assertion also fails. Explicit skips and TODO cases are
recorded separately and block the parity gate, never counted as passes.

Assertion identities use the original output filename plus its occurrence
number within a script. The mapping joins C and Rust receipts; an assertion
observed on only one side becomes `NOT_REACHED` on the other. Runtime failure
checks are kept separately from actual assertions. These are observed
assertions, not a static assertion inventory: even a C baseline can skip a
feature or abort before exposing an assertion. `PASS_SELECTED_SCRIPTS` is
limited to the selected scripts and `rust_complete` is always false.

## Negative checks

`migration/evidence/upstream-harness-negative.json` records intentional
failures through the unchanged `test/graph/00-simple-test` and real
`libtest.sh`: helper exit 23, incorrect output with exit 0, and a sleeping
helper killed by the outer timeout. All must fail. Additional checks reject
a good receipt followed by nonzero exit/timeout, a missing receipt with
exit 0, and a foreign helper route. The self-check also verifies that
`make -Bn RUST_ONLY=1` requests Cargo without C object dependencies,
checks the physical PTY size, and verifies missing-assertion mapping.

## Known semantic gaps

* Rust does not produce the `TIG_TRACE` required by
  `test/main/filter-args-test` and `test/status/file-filter-test`. C reaches three assertions; Rust exits at the
  first trace read. The runner reports this failure and maps the three
  assertions as `NOT_REACHED`; it never manufactures an empty/fake trace.
* A Rust graph helper exists and is now selected explicitly. Successful
  graph fixture assertions cover their output only, not every graph state,
  interactive rendering, C memory checks, or all graph attributes.
* Readline, AddressSanitizer and environment-dependent skips remain gaps.
  Running a C sanitizer target would not certify Rust behavior.
* This adapter does not implement missing UI/config/trace semantics and
  does not convert the original receipt counts into a migration percentage.
  Raw failure details and skipped cases remain in the paired JSON.

## Recorded verification

The full paired snapshot is `migration/evidence/upstream-rust-only.json`,
executed at **91fb1a3** (the harness on top of main **47f1a2b**, including
PR #1). This commit remains in the branch history so its source and harness
hashes can be checked. macOS arm64, Rust/Cargo 1.81.0, Apple Clang 21.0.0.

| Full snapshot | C | Rust only |
|---|---:|---:|
| Scripts attempted | 154 | 154 |
| Passed / failed / skipped scripts | 152 / 0 / 2 | 77 / 74 / 3 |
| Passed actual assertions | 572 | 403 |
| Failed actual assertions | 0 | 152 |
| Assertions not reached relative to C | — | 17 |
| Additional failed runtime/setup checks | 0 | 34 |

Thus the legacy total of **186 failures / 589 checks** is not an assertion
count: 555 assertions were reached, and 34 additional runtime failures were
recorded. The strict script verdict also includes `main/filter-args-test`,
which exited 1 without a receipt. C's 17 assertions not reached by Rust map
to `main/all-arg-test` (4), `main/branch-var-test` (2),
`main/filter-args-test` (3), `main/search-preload-test` (2),
`refs/branch-var-test` (2), and `status/file-filter-test` (4).
All **23 non-sanitizer graph scripts** passed with the Rust helper; the
sanitizer-only application script is explicitly skipped. The parity gate
is **BLOCKED**, and the full run correctly exits 1.

After that snapshot, this branch merged main **3e8f4b8** (configuration
PR #2) without conflicts. The follow-up receipt is
`migration/evidence/upstream-harness-after-sync.json`: C first, then Rust,
for all `test/tigrc/*-test` scripts plus graph, help, default/emoji main,
both trace-dependent scripts, and both diff-context scripts. The full
snapshot above predates PR #2; it is not a claim about the latest full-suite
failure count.

A follow-up guard clears the previous report before running, so an
unexpected runner/build crash cannot leave an old PASS report behind. The
negative self-check deliberately removes `make` from PATH and verifies
nonzero exit plus removal of a seeded PASS report. Empty test selections
also fail. These reporting guards and all earlier negative injections were
rerun after synchronization; original assertions remain unchanged.

Follow-up results are deliberately separate:

| Receipt / source | C scripts | Rust scripts | Rust actual assertions |
|---|---|---|---|
| `upstream-harness-after-sync.json` / **e5cc5b1**, on main **3e8f4b8** | 25 pass | 14 pass, 11 fail | 129 pass, 19 fail; 7 not reached |
| `upstream-harness-after-blame-sync.json` / **7dbceb0**, on main **a28f69d** (PR #4) | 10 pass | 4 pass, 6 fail | 13 pass, 11 fail; none missing |

The second follow-up runs all seven original blame scripts plus graph,
help, and default main. Remaining blame failures are `blob-blame-test`,
`default-test`, `initial-diff-test`, `navigation-parent-test`, `revargs-test`,
and `stash-test`; they remain failed rather than being reinterpreted as
adapter success. Both follow-up runners exit 1 with `BLOCKED`. They are not
a full-suite rerun after either merge.

`upstream-harness-route.json` records an additional Make-route check at
**e5cc5b1**: temporarily move aside both C executables, run
`make RUST_ONLY=1 test/graph/00-simple-test test/help/default-test`, confirm
both scripts and all four original assertions pass without recreating a C
executable, then restore the C files. This proves the ordinary Make route
can run both the application and helper tests without those C binaries.

On the latest synchronized code (**7dbceb0**, main **a28f69d**), formatting,
all **64 Rust unit tests** (46 library + 18 application), Clippy with warnings
denied, and the release build pass. No original assertion or harness library
was edited. All substantive commits made after the attribution instruction
use the requested Codex author and retain the user's committer identity.
