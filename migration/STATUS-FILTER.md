# Status path filter safety verification

Application source is unchanged from main `ac78df21`. The reported display bug
was already fixed there: `status_view` calls `status_filtered` with `file_filter`,
and Git filters each NUL-delimited status query. Selected rows carry their path
into stage/open/unstage. Aggregate opening and refresh use `stage_diff`, which
passes the same active filter into Git. No additional Rust filter is needed.

The existing C/Rust terminal fixture now also retains out-of-path staged and
untracked sentinels, compares their index records and file bytes, and exercises
single-file staging, single-file unstaging, and opening/staging an untracked file.
All 16 checks pass. The original five scenarios remain covered, including the
explicit filter-off case where changing the ordinary outside tracked file is intended.

The unchanged original file-filter, file-name, and untracked-files scripts yield
17 C assertion passes and 16 Rust passes with one failure. `rev-parse.trace#1`
still fails: C classifies argv and queries repository state in a combined call;
Rust directly treats status arguments as paths, discovers the repository using
separate absolute-git-dir/is-bare-repository/show-toplevel calls, and verifies
HEAD with `--verify --end-of-options HEAD^{commit}`. The actual commands are
faithfully traced, including the trailing spaces used by the trace writer.
Changing global repository discovery or fabricating C argv solely to pass this
assertion is outside this status safety slice. This is a documented divergence,
not a passing parity assertion; the strict runner remains BLOCKED.

`evidence/status-filter-final.json` is the single final receipt: source and binary
SHA-256 values, original raw receipts/transcripts, build logs, 16 index checks,
and formatting/83 unit tests/Clippy outcomes. C sources/tests and MIGRATION.md
are unchanged. No full suite was run.

Reproduce after building C and Rust binaries:

```sh
python3 rust/tests/status-filter.py --output /tmp/status-filter-safety.json
python3 rust/tests/upstream-suite.py test/status/file-filter-test test/status/file-name-test test/status/untracked-files-test --output /tmp/status-filter-paired.json
cargo fmt --check
cargo test --all-targets
cargo clippy --all-targets -- -D warnings
```
