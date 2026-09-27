# File finder slice — PASS

Base: `ce3c913b033d328feabdbee3734ba9bdd1b56e0f` (grep integration).
Environment: macOS arm64, Rust 1.81.0. Full migration parity remains OPEN.

`view-blob` with no selected file now opens an incremental file finder;
invoking it from a blob replaces that blob after selection. Cancel and empty
results keep the view. Search-map bindings, resize and TIG_SCRIPT input use
the same selection model. Selection reads the pinned blob OID, never a display
string or shell command. Unix path bytes survive Git NUL records unchanged.
Ambiguous revisions, missing commits and failed object reads are errors.

The pre-integration real PTY probe observed `No selected blob` for
`:view-blob` from main. Its binary SHA-256 was
`01f33d8a04dd0beae3d78e8c8743c03740e4eb1394b34f5af1fcc718e348d24f`.
C supported this interaction; final Rust opens the requested blob.

## Final identity

Rust source SHA-256: `fd86854fe47420d0fa398e18afde83b58e302fbf199e737afb6cddb4747eb25f`.
Algorithm: sorted Cargo.toml, Cargo.lock, tigrc, rust/**/*.rs; append each
relative path, NUL, file bytes, NUL to one SHA-256 digest. This binds the final
source content, independently of the receipt commit.

- Rust target/release/tig: `bbe97e2ef11548b51c5f47eebb99ad0997469a19622deef5b0c937eccc2d2c01`
- C src/tig: `50662705f586850d00f8f4fac2564d13e0013b169e01cd908e6debbbb8917e11`
- rust/tests/file-finder.py: `890c7bfb000bd10c7b33d162b7104e619997c2d94b9fdefb4b8c10df6886f505`
- Strict runner rust/tests/upstream-suite.py: `8070849f857a5cd5a8d42b519c091eaa87c39a43048daca702abd163f74cf702`

## Validation

- `cargo fmt --all -- --check`: PASS.
- `cargo test --locked --all-targets`: PASS, 79 library + 23 application tests.
- `cargo clippy --locked --all-targets -- -D warnings`: PASS.
- `cargo build --locked --release --bin tig`: PASS.
- `python3 rust/tests/file-finder.py`: PASS, 26 controlling-PTY checks:
  C/Rust each 9 common cases; 8 Rust raw-path, empty-result, reopen/cancel,
  script selection/cancel, split history, incomplete-script and resize cases.
- Real Git unit coverage includes non-UTF-8 index-only paths (portable to
  macOS filesystems), tabs/newlines/Unicode/options-like names, symlinks,
  subdirectory invocation, pinned content, ambiguous refs and invalid objects.
- `python3 rust/tests/upstream-suite.py test/blob/wrap-lines-test test/tree/default-test test/tree/chdir-test --output ../finder-upstream.json`:
  C/Rust each 3 scripts, 16 original assertions passed, zero skips/failures or
  missing assertions. Binary routes and unchanged hashes verified by runner.
  Assertion counts: blob/wrap-lines 1; tree/default 7; tree/chdir 8.

Two independent read-only reviews examined safety and behavior. Confirmed
blob/split history, resize and unfinished-script findings were fixed and
regression-tested. A fresh final Reviewer found no remaining material issue;
the security reviewer also rechecked the fixes. No original assertions changed.

## Scope limits

Lists committed blobs (including symlink blobs), like C's ls-tree source;
untracked files and submodule commits are not selectable blobs. Exact C
highlighting, finder screenshots and internal Git trace equality are deferred.
Filtering rescans the in-memory snapshot per edit; large-tree performance was
not benchmarked. No full 154-script run or full-migration completion claim.
