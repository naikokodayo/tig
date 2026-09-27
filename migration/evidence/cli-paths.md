# Native CLI blame paths — 2026-09-27

Scoped PASS, full migration parity remains OPEN. Tested source
`e3fdd84930a0cb44b1688d6c776ad4bddf1a14ee`, rebased on main
`8e92ba33bdf4a77388304a7fa423e09691336285` (mouse PR #72 and search PR #73).
C sources, original tests, dependencies and workflows are unchanged.

`args_os` now reaches CLI parsing. Sequential `-C` directories and blame arguments
retain native bytes; blame converts only options/revisions to UTF-8 and keeps its
single filename as `PathBuf`. Git still receives it after `--`. The existing
GIT_PREFIX, repository validation, blame/navigation and display paths are reused.
No unsafe or global UI string conversion was added. Other views still reject
non-UTF-8 CLI arguments explicitly; native revision/option values remain unsupported.

## Verification

- Rust 1.81.0 on macOS arm64 and Debian Linux aarch64 (Docker's own filesystem):
  fmt, all-target tests **125/125** (97 library, 26 application, 2 integration),
  clippy with warnings denied, release build all pass.
- `python3 rust/tests/cli-paths.py --baseline /path/to/old/tig`: Linux **31 checks**:
  **14 paired C/Rust PTY workflows**, **3 Rust default-config historical navigation
  checks**, **13 rejected combinations**, **1 exact tracked-file/index hash check**.
  It checks native filename bytes, distinct lookalike/special-file content, +line,
  refresh, historical filename/original line, parent/back, leading dash/pathspec/tab,
  native and sequential -C, native GIT_PREFIX and a real Git shell alias.
- Baseline Rust at `521c45fd` rejects the actual Linux byte filename before opening
  a screen. New Rust opens it and navigates to the original byte filename/line.
- `python3 rust/tests/blame-options.py`: **18/18** on both platforms.
- Nine unchanged original scripts: **C 14/14 and Rust 14/14 assertions** on each
  platform, no skips, missing receipts, runtime failures or timeouts. The old Rust
  binary also passes **14/14** on macOS. Scope: `blame/{default,start-on-line,revargs,
  navigation-parent,blob-blame,initial-diff}-test` and
  `{main,diff,status}/start-on-line-test`. No full 154-script run was performed.
- Two independent read-only reviews found no production issue. One identified
  identical special-file test content; unique markers now distinguish every file.
  A fresh final review found no remaining material issue.

## Explicit boundaries and raw exceptions

macOS's local filesystem refuses invalid-byte names; its unit tests exercise
OsStringExt routing, but **real invalid-byte filesystem/PTY proof is Linux only**.
Windows and other filesystems are untested. Filename display need not be lossless;
file identity and Git arguments are. The test refuses unsupported filesystems.

```text
macOS: OSError: [Errno 92] Illegal byte sequence: .../old-\xfe name
Old Rust: tig: Non-UTF-8 CLI arguments are not supported yet; browse the file through the tree/status view
C default core.quotePath=true: tig: No blame exist for "old-\376 name"
```

The C default-config historical navigation failure is retained by the test and
is **not** counted as a passing navigation comparison. Rust's three default-config
navigation checks pass. For the paired historical/parent/back cases, the fixture
sets `core.quotePath=false`, allowing C to consume Git's raw historical filename.
Initial opening/line selection/refresh are compared under the default setting.

Linux setup initially lacked `en_US.UTF-8`, required by unchanged libtest.sh:
C had 8 passing / 6 failed assertions (three scripts), while Rust had 14/14.
After installing/generating that locale, both pass 14/14 without source changes.
Root tar also restored fixture ownership, causing two Rust fixture tests to fail;
command-scoped `TAR_OPTIONS=--no-same-owner` fixes the test environment without
weakening Git's ownership checks. Raw diagnostic examples:

```text
-74537d9 Sébastien Doeraene 2013-10-29 18:46 +0100   4| import scala.scalajs.sbtp
+74537d9 S  bastien Doeraene 2013-10-29 18:46 +0100   4| import scala.scalajs.sbt
fatal: detected dubious ownership in repository at '/tmp/tig-rust-4179-4'
fatal: detected dubious ownership in repository at '/tmp/tig-tree-4179-1790521736120047962'
```

## Source and executable SHA-256

The macOS and Linux source manifests were compared and match. Manifest algorithm:
sort the NUL-delimited output of `git ls-files -z Cargo.toml Cargo.lock Makefile rust
src include compat test`; for each path append its bytes, NUL, and the 32 raw bytes
of SHA-256(file contents), then SHA-256 the concatenation. **342 files**:

```text
d1373e7d968ea297521b168808281547a07ba07b4f825adc31d0ad36bd2c38be  source manifest
fd6b46f7eb78f2f77147776d1d615bae9bea38241feeb8913067827e39f85440  rust/main.rs
366992d4f493ea388af96fc05cc5c01bf521a36dc65ce071ddec55ee0f184579  rust/config.rs
9541a99a6d38a03ba9a9710760a7950c725e68b6282b6051ae92c8210b924dcb  rust/blame_options.rs
99a62c751c8999302385189970a5f167c7ff37728a253a12e495ace821a73873  rust/tests/cli-paths.py
fe8faaad0700832b8dc93ca809a937e27e190aefc9665b935fa5ff45da34800f  macOS C src/tig
d5dd522d9addac44017141c73d09ddf9175160571da3b8969a0825c47b70fbc1  macOS Rust target/release/tig
6ea896a5135c6c89b0ad9a3d7e260d2536540ceb7dd9999035fc6cb548095ab8  macOS baseline Rust
7d04954682197ae14b14f3855801d9153f57c8e3d8d0be021b47ed4680e19308  Linux C src/tig
ea9a9cba32e27394b367e452e300f945e39db8dd073c7be2025521a02c73d4b6  Linux Rust target/release/tig
640cc8fd0bd524711ebd9e31edaeb1da771b3bdf4dfe9bb2e3d5711a61d2db56  Linux baseline Rust
```

Build: `make -j4`; `cargo fmt --all -- --check`; `cargo test --locked --all-targets`;
`cargo clippy --locked --all-targets -- -D warnings`; `cargo build --locked --release`.
Linux used Git 2.39.5; macOS used Apple Git 2.50.1. Baseline source is `521c45fd`.
To reproduce original pairing without generating JSON, import
`rust/tests/upstream-suite.py`, call its `environment`, `routing`, and `run_script`
for each listed script with a 120-second timeout and PATH preferring `src` (C) or
`target/release` (Rust), followed by `test/tools`. Require every result status to
be `pass` and all 14 assertion records to be `OK`; `init.defaultBranch=master` is
provided by `environment`. These are the same original assertion collectors used
by the strict runner, with no modified assertions or generated receipt JSON.
