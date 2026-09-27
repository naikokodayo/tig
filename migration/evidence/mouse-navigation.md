# Mouse navigation — focused receipt

Base: `521c45fd14c82f1f9cd5fa794853a5daca65c5f0`. Code commit before receipt: `6ef28b4c2b746caef6fbf06fa0e51bc8692ad5f0`.
Independent clone; no original assertions, dependencies, unsafe code, or migration rulebook changes.
Rebased over main refresh cancellation (#69), then documentation-only evidence cleanup (#71).

## Behavior and checks

- `set mouse` controls button/SGR capture (default off); live toggle disables capture before exit.
- Content clicks select rows; clicking the selected row enters, except non-stat diff/stage rows. Clicking another pane first changes focus. Reuse drawing coordinates for titles, separators and bounds.
- `mouse-scroll` and `mouse-wheel-cursor` control step and scroll/cursor mode, including C boundary messages, partial bottom viewports and middle-button compatibility.
- Button-only reporting avoids crossterm's hover capture cancelling key prefixes or changing panes.
- `cargo fmt --check`, `git diff --check`: pass.
- `rustup run 1.81.0 cargo test --locked --quiet`: 123 passed (96 library, 25 application, 2 graph); no failures/skips.
- `rustup run 1.81.0 cargo clippy --all-targets --locked -- -D warnings`: pass.
- `rustup run 1.81.0 cargo build --release --locked`: pass.
- `python3 rust/tests/mouse-pty.py`: **28/28 C/Rust real SGR PTY cases pass**. Includes default/live capture, click/second click, horizontal/vertical focus, diffstat, wheel modes/steps/clamps, cursor crossing viewport, blank rows, title/status/separator/outside bounds. Press/release events are separated and C history loading is allowed to finish.
- Oracle compares exported view content/types, dimensions, reference, viewport offset and selected line via `save-view`; terminal bytes separately verify capture and boundary messages. C's cached offscreen per-row `selected` flags are removed; exported `Position ... lineno` is authoritative. This is not a pixel/color screen oracle.
- Original paired runner: **5/5 scripts and 17/17 unchanged assertions pass on each binary**, no skipped or unreached cases: `test/main/default-test`, `test/main/view-split-test`, `test/diff/diff-stat-split-test`, `test/diff/open-after-split-test`, `test/tigrc/save-option-test`.
  Command: `python3 rust/tests/upstream-suite.py <the five scripts above> --output ../mouse-original-final.json`. Verdict `PASS_SELECTED_SCRIPTS`; generated JSON is not committed. Full 154-script suite not run or claimed.
- Two independent read-only reviewers: capture/prefix and boundary-message findings corrected; both final reviews found no remaining material issues. Subsequent partial-viewport guard and blank-row case pass the differential test.

## Before/after

Baseline `0d2b54ff9e65e33d8e2191e2774e3ca65c0501aa` was independently built with Rust 1.81.
`python3 rust/tests/mouse-pty.py src/tig ../mouse-baseline/target/release/tig` exits 1:

```text
AssertionError: ('default capture off', .../mouse-baseline/target/release/tig, 'capture enabled')
```

The final source passes that same default-off assertion and all 28 cases. Raw successful run logs/paired JSON stay outside the checkout; the relevant negative failure is retained above.

## SHA-256

Source aggregate (sorted tracked `rust/`, `src/`, `include/`, `compat/`, Cargo.toml, Cargo.lock, tigrc, Makefile; feed path + NUL + bytes + NUL): `74ceafbb01f9006970e6d2851d1e0830960d809876dd94a48ddc8f4349bddefd`.

- `rust/main.rs`: `69a8b31d5cb95a4e166bca2358fb4d7c20faef8ccb3594ecfe407baf99fe199f`
- `rust/tests/mouse-pty.py`: `927523861360d370f7dd1cf0799fc346c66fa454ebafe5dec746dd3c1ade06aa`
- `src/tig.c`: `1c15c23b26b4aafcf0a9f27e6a605abb701ae75408d801b06d1addd7f87c55f0`
- `src/display.c`: `004164acca5fcee1d0b05fe74a25f8a0089be333096ec9e41797bb82270d730d`
- `src/view.c`: `61d005305e6074d89db95852f426da513e5f75052752b5546f8466964133471c`
- `target/release/tig`: `5bb2c488f0ad60cc95bc4271412988f476e30994ddadb4a501166cba7363aca2`
- `src/tig`: `5866133b11b416518413f9f56105d54740a14fc9e75b8110b82ab32039b93ca2`
- `../mouse-baseline/target/release/tig`: `12dbe45a2d5a024e9f475c69a2b6110b3071dffe0606d6fe476ca254555b632b`
