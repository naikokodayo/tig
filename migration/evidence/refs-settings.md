# Refs metadata/settings receipt — 2026-09-27

Scope: refs mailmap on/off (including live toggle), author/committer date choice,
local date display, ascending/reverse date and case-sensitive identity sorting,
metadata tie breaking. Reuses `parse_history`, `Column::flag`, Chrono and existing
rendering; removes the handwritten timestamp parser. No dependencies added.

Base: `3b721e710199d28f2b81156e04dc815cc86a12bf`; synchronized with `c04c2c90`
(roadmap-only change, no tested source changed). macOS arm64, Rust/Cargo 1.81.

## Identity

SHA-256 over sorted `Cargo.toml`, `Cargo.lock`, `tigrc`, and all `rust/**/*.rs`:
feed each relative path, NUL, file bytes, NUL into one digest.

- Baseline source: `719e234352cc49acd91cb6ef740010a91779125f12491d35677156d769533941`
- Final source: `387a19e25b7a6bffc7fc1868958718ac94b82f9a571433c7ef9e5e50cec83491`
- Baseline Rust binary: `26bc4cc19796842f4b35bc34f0f2638624b6ac37b39f244564b94ad04aacdcde`
- Final Rust binary: `27272d0f92a24b2700c71f194b91cb3bc6618a906f18f32fccea9cd7bd5dd090`
- C binary (`make -j4 src/tig`): `f77d958929042d92aa5407d9988dbc923bb11057370dc0902bb32f9269d430c7`
- New regression `rust/tests/refs-settings.py`: `e372a42148b1bb6b75ad63dcacd320c824b876051449d1d9bcfbcf66325392f2`
- Strict runner `rust/tests/upstream-suite.py`: `8070849f857a5cd5a8d42b519c091eaa87c39a43048daca702abd163f74cf702`

Original C, headers, test scripts, Cargo files, and shared main/git/render modules
are unchanged from the base. The strict runner verified C/Rust binary routing
and unchanged binaries throughout the original-script run.

## Validation

- `cargo fmt --all -- --check`: PASS.
- `cargo test --locked --all-targets`: PASS, 73 library + 26 application tests.
- `cargo clippy --locked --all-targets -- -D warnings`: PASS.
- `cargo build --locked --release`: PASS.
- `python3 rust/tests/refs-settings.py`: PASS, 35 paired real-PTY reference-row
  captures (five mailmap/date configurations × seven captures).
- `python3 rust/tests/upstream-suite.py test/refs/*-test --output ../refs-upstream.json`:
  C and Rust each **9 scripts / 22 assertions passed**, zero skips, missing
  assertions, timeouts or runtime failures. Full 154-script suite not run.

- `test/refs/branch-checkout-test`: C/Rust 7/7 OK
- `test/refs/branch-tag-test`: C/Rust 1/1 OK
- `test/refs/branch-var-test`: C/Rust 3/3 OK
- `test/refs/default-test`: C/Rust 2/2 OK
- `test/refs/filter-test`: C/Rust 2/2 OK
- `test/refs/refresh-test`: C/Rust 3/3 OK
- `test/refs/replace-test`: C/Rust 2/2 OK
- `test/refs/start-on-line-test`: C/Rust 1/1 OK
- `test/refs/worktree-test`: C/Rust 1/1 OK

The PTY regression compares actual reference rows, not the synthetic All row or
status/selection. After live mailmap toggle it compares the same rows unordered:
C resets sorting to ref, while Rust keeps the active sort. That shared navigation
policy in `rust/main.rs` is explicitly outside this slice. Exact sort ties after
repeated sorts, line-number sorting, and mailmap effects in other views remain
outside the demonstrated scope. Full migration parity stays OPEN.

## Red/green evidence

The same regression ran against a separate baseline source archive and binary.
It failed on `mailmap=no` initial display (C raw identity vs Rust mapped identity).
Final source passes all 35 captures. Raw baseline failure:

```text
Traceback (most recent call last):
  File "/Users/liuxu/Documents/Codex/2026-09-27/tig-rust-refs-parity/work/tig/../baseline/rust/tests/refs-settings.py", line 64, in <module>
    assert expected == actual, (mailmap, date_options, name, expected, actual)
AssertionError: ('no', '', 'initial', ['main           2020-01-02 00:00 +0000 Zed        Zed@old.test    same subject', 'alpha          2020-01-01 01:00 +0100 Zed        Zed@old.test    same subject', 'beta           2020-01-03 00:00 +0000 amy        amy@old.test    same subject', 'gamma          2020-01-02 00:00 +0000 Zed        Zed@old.test    same subject'], ['main           2020-01-02 00:00 +0000 Aaron      new@test        same subject', 'alpha          2020-01-01 01:00 +0100 Aaron      new@test        same subject', 'beta           2020-01-03 00:00 +0000 amy        amy@old.test    same subject', 'gamma          2020-01-02 00:00 +0000 Aaron      new@test        same subject'])
```

## Independent read-only reviews

Two fresh-context Reviewer agents reviewed the final source and focused test,
read-only and independently. Correctness review traced loading, mailmap, dates,
ties, filters and callers and reran all 35 paired captures: no material findings.
Design/simplification review traced C metadata/sorting and test coverage: no
material findings. Nonblocking coverage limit: the separate
`refs-view-date-use-author` override is not exercised by the PTY regression.
No source changes were required after either review.
