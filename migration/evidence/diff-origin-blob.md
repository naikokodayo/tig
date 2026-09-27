# Diff and stage historical blobs

Source commit: `5890e6a5`, based on main `dc16e3cc`. The following evidence-only
commits do not change the tested source.

Diff `view-blob` now shares the blame-origin chain: deleted lines use the parent,
added/context lines use the displayed commit, and blame supplies the historical
path and original line. Diffstat opens the displayed revision directly. Stage
opens HEAD at the old-side line, clamped to the blob length. Existing source/display
mapping handles wrapping. Git parsing, path validation, and blob loading are reused.

- Original C/Rust scripts: 4/4 each, 18/18 assertions each. No skips or weakened assertions.
  Scripts: `test/blame/blob-blame-test`, `test/blame/initial-diff-test`,
  `test/diff/wrap-lines-test`, and `test/stage/default-test`.
- Real Git/PTY regression: baseline C 8/8, baseline Rust 0/8; final C and Rust 8/8 each.
  Eight scenarios: deleted origin, added origin, context across a rename, diffstat,
  root addition, staged deletion, staged EOF addition, and staged addition.
  Each passes in both implementations; index tree and worktree bytes remain unchanged.
  Reproduce with `python3 rust/tests/diff-origin-blob.py --output /tmp/diff-origin-blob.json`.
- Rust 1.81 release build, 25 application tests, formatting, Clippy and diff checks pass.
- Three independent read-only reviewers inspected the final scope; no material findings.

Source manifest SHA-256 (sorted compact JSON mapping tracked Rust/Cargo paths to SHA-256):
`d2e9e9c7926489d8c1af13cb423b905c67ce07baa007faafcbdab5729c7cbdb7`

Release `tig` SHA-256:
`1c377cecefad00520762b68a3608737af1595118e6b9d7e8612b3ab9d02552cf`

The detailed baseline and selected-script receipt is archived in commit `8900850a`;
this compact summary replaces the working-tree JSON snapshot. It does not claim
a full 154-script checkpoint or completion of the Rust migration. Original C,
tests, attribution, dependency versions, unsafe prohibition and MSRV are unchanged.
