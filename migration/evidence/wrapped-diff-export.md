# Wrapped diff rows and save-view — focused receipt

PASS for the scoped display/export chain; full migration parity remains OPEN.
Tested code: `f612cd97e5b9078e05049f8b4f15346449ea6634` on latest main `68a1be2b` (2026-09-28), macOS arm64,
Rust/Cargo 1.81.0. The following receipt commit changes documentation only.

Source row types/cells now drive both wrapping and export. Diff-stat, hunk and
inserted refs boxes remain single rows, matching C's direct-box paths; ordinary
continuations retain their source type. Displayed selection flags and reference
state are recorded during drawing. Navigation/editing still use source rows.
Exclusive-create remains in place. Word-diff, custom colors, ANSI cells and
unsupported wrapped non-diff exports still fail closed; no general renderer was added.

## Before and after

Baseline main `68a1be2b` was built from a Git archive with Rust 1.81.0.
`python3 rust/tests/diff-view-export.py --before --rust-binary ../wrapped-baseline/target/release/tig`
passes its negative oracle: C exports both ordinary wrapped and long-cell diffs;
Rust exits 1, creates no destination, and reports:

```text
tig: save-view does not support wrapped diff views yet
```

`python3 rust/tests/diff-view-export.py` passes **13 cases** with no timeouts:
9 successful byte-for-byte C/Rust export comparisons and 4 expected Rust safety
errors. At 40 columns, the Git-generated long diff-stat filename and long Python
function hunk context retain exact typed cells. Their wrapped, selected and
unwrapped screen snapshots also match C. Ordinary wrapped added content exports
four diff-add rows. In-view and offscreen cursor movement exercise stored flags.
The existing-destination case returns an error and preserves `untouched`; custom
color cases retain their prior rejection. C intentionally overwrites those files.

## Checks

- `rustup run 1.81.0 cargo fmt --all -- --check`: PASS.
- `rustup run 1.81.0 cargo test --locked --all-targets`: PASS, 125 tests (97 library, 26 application, 2 integration).
- `rustup run 1.81.0 cargo clippy --locked --all-targets -- -D warnings`: PASS.
- `rustup run 1.81.0 cargo build --locked --release`: PASS.
- `python3 rust/tests/wrap-rendering.py`: PASS, 8 existing PTY checks for titles and source-line blame tracing. The script retains its documented Rust safety difference for wrapped blame tracing; these are not eight blanket parity claims.
- Independent read-only review of the final source: no material findings.

`rustup run 1.81.0 python3 rust/tests/upstream-suite.py <scripts> --output ../wrapped-originals.json`
passes **7/7 scripts and 11/11 original assertions on each C/Rust side**.
No skipped, unreached or failed assertions, runtime failures or timeouts:

- `test/blob/wrap-lines-test` (1 assertions each side).
- `test/diff/commit-title-wrap-test` (1 assertions each side).
- `test/diff/diff-stat-split-test` (1 assertions each side).
- `test/diff/diff-stat-test` (1 assertions each side).
- `test/diff/line-number-test` (1 assertions each side).
- `test/diff/wrap-lines-test` (1 assertions each side).
- `test/script/default-test` (5 assertions each side).

The paired runner routes C to `src/tig` / `test/tools/test-graph`, Rust to
`target/release/tig` / `target/release/test-graph`. The focused PTY checks use
explicit binary paths and disposable Git fixtures. All final binary hashes were
verified after running the unchanged original scripts. No full 154-script run;
no original C/tests, dependencies, MIGRATION or ROADMAP changes. Generated JSON
remains outside the checkout and is not committed.

## SHA-256

Source manifest: sorted tracked Cargo.toml/Cargo.lock, Makefile, tigrc, rust/,
src/, include/, compat/, test/ and tests/ paths; concatenate UTF-8 lines
`<file SHA-256>  <path>\n`, then hash that manifest.

- Source manifest (346 files): `0837b36aa160ec7470cdc2093fb77aedacc9650161499702a1d30bc9bea3109c`
- `rust/main.rs`: `b4b4c88d21f0da52c7879d8671840e3c589ba96f3e0907d7af8cb45cc378d376`
- `rust/render.rs`: `527340b05ebeb478ec2b1992e68c29265ead2b89e9192879676f9cd0c2efa05d`
- `rust/tests/diff-view-export.py`: `a974951db47e3f07c006f958dc4291f1be2460c386211c6a4276ca918110ff5a`
- `rust/tests/wrap-rendering.py`: `551b8200fea5d62e92a676b1b28c428ef57854449c83fb5bd5459497daaaec5e`
- `rust/tests/upstream-suite.py`: `40f68213956d46c2611e10ac1ebe48c72d1bb6a24443705a2db452d92d1a7663`
- `src/tig`: `ca291ff2ccb34aeb6cdb1b1ad6c8ff71ab821de309cef2f8ac6c3b48a112ab25`
- `test/tools/test-graph`: `48bf41175c4a603ec9a5b335fdc8e7f2f0f90b79b99df7a509202686880b7849`
- `target/release/tig`: `62fe4e5747a3934e3fc181893df71784bbb40626e7cdc2e69a30941b907e8845`
- `target/release/test-graph`: `ea37b3ab5eccb5920e3ec9f947197fb83b986630f7507168e0b103ff0bf4b5f3`
- `../wrapped-baseline/target/release/tig`: `5c95861f49de9ddc7774bf4e359659c061b75d52b663b9d2727178af7a1cdf23`
