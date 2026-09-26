# Rust tigrc save-options slice

Based on `34684f799a5b44e7074aa4967c53c10608119f10`, including PR #2.
The Rust migration remains incomplete. No upstream test or expected output changed.

## Cause and change

`append-option-test`, `save-option-test`, and `builtin-save-test` reached an
unimplemented `save-options` action. Append validation itself already worked.
The unchanged append test initially recorded four failures, including two process
errors and missing output, with `Not implemented in Rust yet: save-options`.

The shared script/interactive action now saves the current Rust configuration to
the requested path, defaulting to `tig-options.txt`, and reports success or I/O
failure in the status line. Files are created exclusively, matching C's `O_EXCL`;
existing files and symlink targets cannot be overwritten. Output is validated
before creation, and write/sync errors are reported.

Serialization includes stored settings (excluding transient `*-args`, like C),
bindings, and colors. It retains argument boundaries, empty values, quotes,
backslashes, special keys, and binding order within request/command groups.
Colors use C's named-area versus quoted-prefix syntax. A saved default config was
also loaded by the existing original C build: exit 0, empty stderr.

## Verification

On macOS arm64, with Rust 1.81.0:

- `rustup run 1.81.0 cargo fmt --check`: pass.
- `rustup run 1.81.0 cargo test --locked --all-targets`: 65 tests pass.
- `rustup run 1.81.0 cargo clippy --locked --all-targets -- -D warnings`: pass.
- `rustup run 1.81.0 cargo build --release`: pass.
- `python3 rust/tests/terminal-smoke.py`: 111 checks pass, including quoted save
  paths, exclusive creation, and missing-directory errors.
- All 17 unchanged `test/tigrc/*-test` scripts were run under a controlling PTY,
  with `target/release` and `test/tools` prepended to PATH and Git's initial branch
  pinned to `master`: 97 OK and 15 FAIL records. The final color serialization
  adjustment was followed by rerunning the three save scripts, all passing, and
  the full Rust/PTY checks. Other tigrc script results precede that adjustment.
- The three save scripts have 5 passing assertions in total. The builtin test's
  default `work-dir/tig-options.txt` was also confirmed to exist. The original C
  build passes unchanged append/save scripts as a reference.
- The new Rust save regression was observed failing before implementation. It
  now checks full stored-state round trips into empty/default configurations,
  repeatable serialization, special arguments/keys/colors, non-overwrite, and
  rejection before file creation of unrepresentable arguments.

Counts and the final release binary hash: [tigrc-save.json](tigrc-save.json).
The full 154-script upstream suite was not rerun; record counts are not a
migration completion percentage.

## Still unresolved

| Original script | FAIL records | Observed gap |
| --- | ---: | --- |
| command-value-long-test | 1 | Command output does not open the expected pager view. |
| compat-error-test | 1 | Obsolete option/key diagnostics and recovery differ. |
| escape-var-test | 2 | `refname` command variable is unsupported. |
| quote-test | 6 | Prompt-variable quoting reaches `Unclosed command variable`; subsequent screens are absent. |
| truncation-test | 4 | Grep file-name rendering does not honor literal/UTF-8 truncation delimiters. |
| view-column-test | 1 | Invalid column error text differs from C. |

These paths were inspected but left for separate slices; command execution and
prompt-variable expansion are unchanged.

This saves the state represented by Rust's current `Config`, not a byte-for-byte
copy of C's output or a complete materialization of C's implicit defaults. The
existing color model does not retain the lexical distinction between a quoted
literal matching a built-in area name and that named area. Saving uses the named
area in that ambiguous case. Existing default/system bindings still overlay when
a saved file is subsequently loaded; this does not introduce a reset directive.
Literal `#`, CR, or LF in arguments cannot survive the existing config file parser;
saving rejects those explicitly instead of silently truncating them. No shell is
used to serialize or save commands, and saved commands are not executed.

The PR is intended for independent review and merge by the main task.
