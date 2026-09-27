# Search result count and highlight receipt

Tested source commit: `f94e4a6d1dcf9b3b1da457265031760dc12e612f`, rebased onto main `672ebbe5a8bf97bb449b51b82446e39667ed5bd4` (includes PR #69 refresh cancellation and PR #72 mouse navigation).
macOS arm64; Rust/Cargo 1.81.0. First-party unsafe remains forbidden. Original C sources and assertions are unchanged.

## Delivered behavior

Search navigation reports matching-line ordinal and total (multiple spans on one row count once), forwards/backwards with optional wrap. A per-view compiled regex drives both row eligibility and visible span highlighting. Highlighting follows rendered/clipped text, stops on empty matches like C, and uses configured global/view `search-result` colors and attributes; nonempty `NO_COLOR` suppresses colors. Refresh clears highlights until the next find, as C does.

Main-view search retains formatted source column values and individual reference names, so generated line numbers, graph cells and padding cannot create matches. Date/author formatting is reused. No new dependency; the replaced row scan and refs-only helper are removed.

## Checks

- `rustup run 1.81.0 cargo fmt --check`: pass.
- `rustup run 1.81.0 cargo test`: 124 tests pass (96 library, 26 application, 2 watch).
- `rustup run 1.81.0 cargo clippy --all-targets -- -D warnings`: pass.
- `rustup run 1.81.0 cargo build --release`: pass.
- `python3 rust/tests/search-pty.py src/tig target/release/tig`: **16 paired status/highlight snapshots pass**, plus invalid-regex failure clears highlights on both. Covers repeated spans, next/previous/empty search, forward/backward wrap, no-wrap, absent matches, ERE classes/repetition, case-sensitive/ignore/smart case, wrapped blob lines, refresh, generated line-number exclusion and anchored source-title matching. The test compares actual colored terminal cells, not just saved plain text.
- `python3 rust/tests/main-refresh-pty.py target/release/tig`: all four existing cancellation/reap/retain/retry/failure/quit checks pass after rebase; R/z response 0.098 seconds in this run.
- `python3 rust/tests/mouse-pty.py src/tig target/release/tig`: **28/28 real C/Rust SGR mouse cases pass**, including both split layouts and diff clicks.
- `rustup run 1.81.0 python3 rust/tests/upstream-suite.py test/main/search-test test/main/search-preload-test test/blob/wrap-lines-test test/diff/wrap-lines-test --output ../search-final-upstream.json`: **4/4 scripts, 12/12 original assertions on each side**, zero failures/skips/unreached assertions. C routes to `src/tig`; Rust routes to `target/release/tig`; both graph helper routes are verified by the runner. Generated JSON remains outside the tracked checkout. No full 154-script run claimed.

## Before/after and test sensitivity

Baseline source `0d2b54ff9e65e33d8e2191e2774e3ca65c0501aa` was built separately using Rust 1.81.0. The same PTY test failed on its first Rust search. Exact decisive assertion payload:

```text
'/needle\r', ('', {})
```

That is an empty status line and no highlighted cells; expected `Line 2 matches 'needle' (1 of 2)` and 18 highlighted characters. C passes that oracle, and the final Rust passes every paired snapshot. During refresh validation, C returned `('', {})` after R; the final test deliberately asserts that highlight-clearing behavior rather than preserving stale results.

## Remaining boundary

This is the requested count/highlight slice, **not full POSIX regex parity**. Existing Rust regex syntax remains: POSIX leftmost-longest alternation, backreferences, locale-dependent classes/case behavior, and non-main field-specific C grep extraction require a later focused slice. A reviewed ref-name boundary remains: C preserves `refs/tags/x` when any branch named `x` exists, while current decoration data only yields the short tag name. Repository-wide branch/tag collision handling needs typed reference metadata and is explicitly not a pass here. Ordinary notes/stash/prefetch/replacement name normalization is covered by the updated Rust unit check. Tested ASCII case modes and common ERE classes/repetition match C. Split-pane coordinates and UTF-8 boundaries were reviewed; byte-span Unicode/empty-match behavior has a unit check, but split-layout/ref-only matches and non-ASCII terminal-cell parity are not claimed by this PTY fixture. The original Unicode-author search script passes.

## Review

Independent read-only reviews identified and prompted fixes for generated main line numbers and ordinary reference prefixes. The repository-wide branch/tag name collision case above remains explicitly deferred within the user-authorized bounded slice.

## SHA-256

Source-set hash: `1f6de4adb5d0c5b69f964492a4edb6dbc284807413e7b52ed67fb32be48faa5c`.
Recipe: sorted Cargo.toml, Cargo.lock and all rust/**/*.rs paths; SHA-256 over each UTF-8 relative path, NUL, and its raw 32-byte SHA-256 digest. This excludes this receipt and generated evidence.

- `rust/main.rs`: `c89ccfc748c8f5c9fc88345e54ce83de2caa77b67614f3f61861718a2b27c4a8`
- `rust/render.rs`: `f41aa97fb5a0886a860ce1a98599748accc72b61b899b558cde3864e5e94fa0c`
- `rust/tests/search-pty.py`: `d4468237010b30c368399a9c4ede1a6db15470e2bd1b418e9a05fb86f6cd32e6`
- `src/search.c`: `1b4f90f7ecb94d93ad631728237fab9a5955559b163eef29d541bcc468c74fa5`
- `src/draw.c`: `a6f87bc1e2451194aecafd385bd088e9b24923b245ac36a0b538d4ee5cecb4c3`
- `test/main/search-test`: `890a9529f8b868916427fb5302e0b0dbe18fc183f68fe5dcba441c2839250d41`
- `src/tig`: `5866133b11b416518413f9f56105d54740a14fc9e75b8110b82ab32039b93ca2`
- `target/release/tig`: `28ac548eb29d543d96d5e45e08a329e824f405543cc16ee78763b03672b1a8e4`
- `target/release/test-graph`: `51a8cb5d8f0ab6147df176d419e80d9983859c67a81d1d0b46f8d39595c5af7f`
- Baseline Rust binary: `12dbe45a2d5a024e9f475c69a2b6110b3071dffe0606d6fe476ca254555b632b`
