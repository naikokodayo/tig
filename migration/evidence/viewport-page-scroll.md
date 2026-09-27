# Page viewport scrolling — focused parity receipt

Verdict: PASS for this slice; full migration parity remains OPEN. No full 154-script run.

Tested source: `bbab926fdadd5aed43b8e78a398d3c4edbff2789`, rebased onto `f670b09e` on 2026-09-27.
macOS arm64; Rust/Cargo 1.81.0. Only Rust dispatch, the focused regression,
and this receipt change. Original C/tests and dependencies are unchanged.

The four page/half-page actions reuse the existing line/wheel scroll path:
active pane height supplies the step, both viewport and cursor move by the
clamped delta, and existing first/last-line reporting remains intact.

## Validation

- `rustup run 1.81.0 cargo fmt --all -- --check`: PASS.
- `rustup run 1.81.0 cargo test --locked --all-targets`: PASS, 125 tests (97 library, 26 application, 2 integration).
- `rustup run 1.81.0 cargo clippy --locked --all-targets -- -D warnings`: PASS.
- `rustup run 1.81.0 cargo build --locked --release`: PASS.
- `python3 rust/tests/viewport-scroll.py`: PASS, 167 paired states across six real Git/PTY scenarios. Explicit C/Rust binary paths; all runs exit 0 without timeout. Checks exported viewport, selected line, dimensions and return view for long pager/blob, horizontal/vertical child and pager-parent panes; odd heights, nonzero cursor row, partial and exhausted top/bottom bounds. Fixture remains clean. This compares exported state, not every terminal pixel or boundary message text.
- `python3 rust/tests/pane-layout.py`: PASS, 17 C/Rust screen comparisons.
- `python3 rust/tests/mouse-pty.py`: PASS, 28 C/Rust real SGR mouse cases; existing line/wheel scrolling remains covered.
- Two independent read-only reviews: no remaining findings. Parent fixture corrections preserve actual split/return behavior; production scope remains 12 added lines.

Original unchanged scripts, via `rustup run 1.81.0 python3 rust/tests/upstream-suite.py <scripts> --output ../scroll-upstream.json`: C **5/5 scripts, 26/26 assertions**; Rust **5/5 scripts, 26/26 assertions**. No skipped, missing, failed or runtime-failed checks. The temporary JSON is not committed.

- `test/blob/wrap-lines-test`: 1 assertions on each side.
- `test/main/goto-test`: 8 assertions on each side.
- `test/main/jump-ends-test`: 8 assertions on each side.
- `test/main/view-split-test`: 4 assertions on each side.
- `test/script/default-test`: 5 assertions on each side.

## Negative baseline

Same focused regression against the unchanged `40187436` release binary exits 1;
C reaches the checkpoints but Rust stops at the first unsupported command:

```text
('pager', 'rust', 1, 'tig: Not implemented in Rust yet: scroll-page-up\r\n')
```

## SHA-256 provenance

Source manifest covers tracked Cargo manifests, Makefile, tigrc, rust/, src/,
include/, compat/, test/ and tests/. It is sorted by path and consists of
`<file SHA-256>  <path>\n` lines; hash that UTF-8 manifest to reproduce the aggregate.

- Source manifest (345 files): `d51cce1fc80dea91eb485d59fc73865d4f0b5c88a40ce16e1f43352390becb35`
- `rust/main.rs`: `5654756dc6b94e2dd7d98ac38c0cd21ca978026abfdcad594e65333f7ae26243`
- `rust/tests/viewport-scroll.py`: `585cecbcc9e64f4d616aebac98ef093e920669b2afc09c0a2b625488d8814c4a`
- `rust/tests/upstream-suite.py`: `40f68213956d46c2611e10ac1ebe48c72d1bb6a24443705a2db452d92d1a7663`
- `src/tig`: `ca291ff2ccb34aeb6cdb1b1ad6c8ff71ab821de309cef2f8ac6c3b48a112ab25`
- `test/tools/test-graph`: `48bf41175c4a603ec9a5b335fdc8e7f2f0f90b79b99df7a509202686880b7849`
- `target/release/tig`: `eb2c5077c708bb67d2caff85f14454c11b998cfa951fd7f56c31068c181ecad2`
- `target/release/test-graph`: `98c94aabaf2a9c42d54077caa3b57ed77978e03172fa208f2828cd778d6b0b73`
- `../tig-before`: `d5dd522d9addac44017141c73d09ddf9175160571da3b8969a0825c47b70fbc1`
