# Interactive options menu — focused receipt

Base: `9b7fbe073f8d8769a04c0f656eb28e4d498566e0` (pre-change test baseline). Synced before commit to `c5c7dd90` (documentation-only main update; Rust inputs unchanged).
Redesign slice: real terminal `o`, `:options`, and bound options requests dispatch the 18 C menu choices through existing Rust toggle/refresh. No dependencies, unsafe, license or original assertion changes.

## Validation

- `cargo fmt --check`, `git diff --check`: pass.
- `rustup run 1.81.0 cargo build --release --locked`: pass.
- `rustup run 1.81.0 cargo test --locked toggle`: 3 passed.
- `python3 rust/tests/options-menu-pty.py src/tig`: 10 passed.
- `python3 rust/tests/options-menu-pty.py target/release/tig`: 10 passed.
  Covers default key, colon command, hotkey, four arrow directions, wraparound, unknown input, Escape and Ctrl-C; saved settings must equal direct toggle (or unchanged cancellation).
- Same PTY regression against a release build of base: fails case 0 at `assert b"Toggle option line numbers" in actual[0]`. This is the missing-menu baseline, not a waived failure.
- `python3 rust/tests/upstream-suite.py test/main/default-test test/tigrc/save-option-test --output ../options-original.json`: C 2/2 and Rust 2/2 scripts, 8/8 original assertions each, no skips; `PASS_SELECTED_SCRIPTS`. Raw successful temporary runner receipt is not committed. Full 154-script gate not run or claimed.
- Independent read-only behavior review: no material findings. Independent simplicity/correctness review found count alignment; corrected to C right alignment and verified resolved. Final fresh review: no material findings.

Scope limit: PTY checks compare the menu to each executable's existing toggle behavior, not all setting implementations or TIG_SCRIPT menu support. Original scripts separately protect existing render/toggle behavior.

## SHA-256

Source aggregate (sorted tracked `rust/`, Cargo.toml, Cargo.lock, tigrc, plus new PTY test; feed path + NUL + bytes + NUL): `a2a8fb997419db687cdc58405bf6e372689da5cd98c920a1dce55ffed165b2a5`.

- rust/main.rs: `97b24368dbf111ccbdaa563d95afd11ee8c42f6c2009d7e90830d81cbcfed8ea`
- rust/tests/options-menu-pty.py: `4792d8d94ae02ce4ad58529d1e68105991e561ae0af9921b2043ef32b3ae0e6d`
- Rust release target/release/tig: `89a402434136ce396c9430c1a35f4b73e7e2f9ce33500ac30950250eb68f0554`
- C src/tig: `16dbbda87e0619f32cb959bd6dd60725d2f4657c7773243d283d42fd3bb52345`
- Base Rust release: `b7a0724d7ffb26c83b68bc9a68c9f57d4325d882a064cb05ad86a30a42962974`
