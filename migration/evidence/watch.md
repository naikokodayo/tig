# Watch / automatic refresh slice

Integrated base: `a9724d1e7cae6937207fd255becffa5557cb88e6`. Rust 1.81; original C and tests remain unchanged.
No dependency, Git backend, Actions, or full-suite receipt changes.

## Behavior

The input loop polls repository state only in periodic mode. Manual mode does
not refresh after external commands; after-command and auto retain command
refresh. Periodic uses the configured interval (zero/negative disables polling).
Resize redraws existing data in manual mode; existing pane rendering rewraps
text. Prompt/finder interactions retain their existing blocking input behavior.

HEAD, refs (including packed/shared worktree refs), stash reflog, status, and
tracked diff content are compared through the existing Git backend. Optional
index writes and external diff/text conversion are disabled in watch queries.
Same-status tracked edits and dropping an older stash are detected. Untracked
content-only edits with unchanged names are not detected, as in C's watcher.
Queries remain synchronous and buffer Git output like existing loaders.

Baselines are taken before view reads, preserving later concurrent changes.
A failed pane does not block the other pane; failures retry at the next deadline.
Returning to a cached view in periodic mode reloads it. A single-pane explicit
refresh intentionally does not acknowledge a global baseline: it may cause one
conservative extra periodic refresh rather than hide changes in another view.

## Validation

- Rust 1.81 fmt, 109 tests (84 library, 23 UI, 2 real-Git watch), Clippy with
  warnings denied, and release build passed.
- Watch tests cover unborn HEAD, staged/unstaged changes, repeated same-length
  edits, stash creation/deletion including older entries, packed refs, branch
  switching, linked worktrees, bare repos, unchanged index bytes, deadlines,
  disabled polling, errors, and retry-until-success acknowledgement.
- Real PTY: manual, after-command, periodic each passed idle external commit,
  command-return, and explicit-refresh assertions. The pre-integration binary
  failed periodic idle refresh and incorrectly refreshed manual after a command.
- Existing fullscreen grep command refresh: 2 checks passed. File finder:
  26 checks passed during integration. Status revert: 8 checks passed.
- Four unchanged original scripts: C 27/27 and Rust 27/27 assertions, no skips,
  missing assertions, or runtime failures; selected gate PASS.
- `test/main/refresh-periodic-test`: C 5 / Rust 5 assertions, all OK.
- `test/main/refresh-test`: C 8 / Rust 8 assertions, all OK.
- `test/refs/refresh-test`: C 3 / Rust 3 assertions, all OK.
- `test/status/refresh-test`: C 11 / Rust 11 assertions, all OK.

Two independent read-only final reviewers approved after corrections for older
stash deletion, failed-pane retries, hidden-view restoration and baseline timing.
No full 154-script rerun or completed-parity claim is made.

Reproduce:

```sh
rustup run 1.81.0 cargo test --locked --all-targets
rustup run 1.81.0 cargo clippy --locked --all-targets -- -D warnings
rustup run 1.81.0 cargo build --locked --release
python3 rust/tests/watch-pty.py target/release/tig
python3 rust/tests/upstream-suite.py test/main/refresh-periodic-test test/main/refresh-test test/status/refresh-test test/refs/refresh-test --output /tmp/tig-watch-upstream.json
```

## SHA-256 source and binaries

- `rust/main.rs`: `0e204cc2ddbaadb784b4ed484a90e189137f8c4b915dd2bda58c4815edc3cdb3`
- `rust/lib.rs`: `f60c67e4bce3061ff398a4b716ae22fe1cd50c49cc7fddd1fd1fe7224e167562`
- `rust/watch.rs`: `851c479af44000c515ee5949416cb3f114f62b1b98c610d967d4e8fab296c81c`
- `tests/watch.rs`: `c278433a8eb7a01076f848e90061e1674adc5047928c95a1b463f514379f2efa`
- `rust/tests/watch-pty.py`: `2547e0be6a00c6506784ef1b696b1615db25dbcc6e7250ce6cca529d6be378d4`
- `target/release/tig`: `d4bd1bde87499723234b48a9272f6af7d1d7d2d80c97cb0e3096868fd821fa79`
- `src/tig`: `6457eddca79bfda4741bdb992fd207b5df0ecf531c22aaaebf91988fcec97041`
