# Rust diff context slice

Scope: the unchanged upstream `test/diff/diff-context-test` and
`test/diff/diff-wdiff-context-test`, against the safe Rust executable.
Reference C behavior: `src/options.c` (`update_options_from_argv`,
`diff_context_arg`, `word_diff_arg`) and `src/diff.c` (`diff_open`,
`diff_save_line`, `diff_restore_line`).

## Root cause and change

The diff loader called a fixed `Repository::show`, so configuration changes
never reached Git. CLI `--word-diff` stayed in history arguments, where it was
rejected on returning to main. Refresh retained screen indices instead of the
selected file's source line.

Pass context and plain/none word-diff mode explicitly to Git; consume the
supported shared CLI switches (`-U<N>`, `--word-diff`, `--word-diff=plain`,
`--word-diff=none`) for main/diff startup, respecting delimiters and history
option values. Keep the resolved commit ID in the diff view and restore
ordinary patch selections by file and new-file line, keeping their screen row.
No new dependency, unsafe code, or original-test change.

## Validation

Local platform: macOS arm64. Toolchain: Rust 1.81.0.
Validation code base: `e94a87b43eba520d0c0e54d1dd19c3d6ddd367b6`.
Before committing, advanced to latest fork main `4b2262a7b0dd7c3b0a62316a5ecb4e6582caaade`;
that intervening commit changes only the roadmap documentation.
The failing baseline was measured at `439b13a`; no original test was edited.

```sh
rustup run 1.81.0 cargo fmt --all -- --check
rustup run 1.81.0 cargo test --locked --all-targets
rustup run 1.81.0 cargo clippy --locked --all-targets -- -D warnings
rustup run 1.81.0 cargo build --locked --release
# Run each original script in a terminal (the harness reads /dev/tty).
PATH="$PWD/target/release:$PWD/test/tools:$PATH" sh test/diff/diff-context-test
PATH="$PWD/target/release:$PWD/test/tools:$PATH" sh test/diff/diff-wdiff-context-test
```

- Format, all-target tests (40 library + 16 application), Clippy with warnings
  denied, and release build: PASS.
- Original diff-context: 10/10 assertions PASS; baseline was 1/10.
- Original diff-wdiff-context: 10/10 assertions PASS; baseline was 0/10, plus
  an unexpected exit status when opening main.
- Both original scripts exit 0 after this change. No remaining failing
  assertions in these two scripts.
- Each script checks `diff-default`, `diff-u4`, `diff-u5`, `diff-u10`, `diff-u8`
  and each corresponding `-from-main` screen.
- New Rust regressions cover CLI option/path/value boundaries, context 0/3/4/5/8
  in both word modes, deletion-line restoration, missing targets, file boundaries,
  and conservative fallback for combined diffs.

## Independent differences and limits

The upstream assertions use `git diff --no-index -w` by default. Passing them
is not byte-for-byte screen parity: all ten ordinary-diff screens retain a
space on blank context rows; the word-diff `diff-u4.screen` and
`diff-u4-from-main.screen` have two-space content indentation where their
expected snapshots have three. These are whitespace-only differences; the
other eight word-diff screens match byte-for-byte.

Combined-diff source-line restoration is not added; it retains the existing
screen-index fallback. This slice does not implement `--word-diff-regex`,
color/porcelain modes, all `diff-options` effects, or all show path filters.
The full upstream suite was not rerun here. The migration remains IN PROGRESS;
this evidence covers only the stated slice.
