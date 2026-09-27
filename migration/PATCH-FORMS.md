# Partial patch forms: executable mode with text

At base `5fb47149` (same Rust/C sources as `a62eaa5b`), the C implementation
copies every file header before applying a selected line, block, or hunk
(`src/stage.c:stage_diff_write`, `stage_apply_line`, `stage_apply_part`,
`stage_apply_chunk`). Rust's `Patch::select_range` previously rejected every
`old mode` / `new mode` header. All six public executable-bit/text cases passed
on C and were explicitly refused by baseline Rust.

This slice permits regular-file `100644` / `100755` mode headers with text
selections, retaining the C behavior: the executable-bit change accompanies
the chosen text. It does not try to separate mode and text into independent
operations. Non-regular `old mode` / `new mode` headers remain refused. Path validation, Git's cached
preflight, atomic apply, and worktree isolation remain in the same shared
apply path. No `rust/main.rs` or C source changes are required.

## Observed boundaries

Real-index probes used a committed six-line regular file, then selected one
changed line from status → stage. Both executables retained worktree bytes
and permissions in all probes. A C zero exit is not counted as evidence of a
successful index operation; index snapshots decide the observation.

- Added file (`git add -N`, or fully staged): C made no index change in these
  single-line probes. Rust explicitly refuses added-file line/block selection.
- Deleted file: C made no index change when staging one deletion; unstaging
  one deletion restored only that line into the index. Rust explicitly refuses
  both line directions. Full text-hunk selection is already available.
- Staged rename plus appended text: C unstaged the rename and selected text
  together. Rust's selected-path diff presented the destination as an added
  file and refused line selection. A raw rename/copy patch is also explicitly
  refused by `select_range`; rename support needs a separate path-aware slice.
- Binary and mode-only forms have no selectable text hunk: the C line request
  made no index change; Rust explicitly refused it. A reviewer additionally
  checked mode-only `status-update` from the stage view: C stages/unstages the
  mode, while Rust refuses the no-hunk patch. That preexisting gap remains.
  Whole-file status staging
  uses `Repository::stage` / `unstage` and is outside partial-patch selection.
- Regular executable-mode change plus text: C and new Rust stage/unstage the
  selected first line/block/hunk plus mode, leaving the second hunk untouched.
  Mode `120000`, `160000`, and invalid `100600` headers are refused by the new
  focused unit regression. This is not a claim of symlink/submodule parity.

## Verification

`rust/tests/patch-mode.py --output <receipt.json>` runs 12 public C/Rust cases:
line, block, hunk × stage/unstage × C/Rust. It checks exact index bytes and
mode, unchanged worktree bytes/permissions, successful exit, and no timeout.
The Rust unit regression also reapplies a stale selection, checks byte-for-byte
index preservation on failure, and rejects non-regular mode headers.

The compact source/binary-bound receipt is
`migration/evidence/patch-mode.json`. It includes baseline refusals, final
public cases, form observations, and the unchanged original
`test/stage/default-test`, `test/stage/update-part-test`, and
`test/stage/split-chunk-test`: 3/3 scripts and 21/21 assertions per executable
both before and after. Formatting, 96 Rust tests, and Clippy with warnings
denied pass. Independent read-only review found no new safety issue and identified the
preexisting mode-only stage-view gap above.
This is scoped evidence; the full migration parity gate remains open.
