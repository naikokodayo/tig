# Prompt completion slice

Base: `0406888bd37fef9e980824ad22723e998e8ee230` (merged main). The unchanged upstream `test/main/search-preload-test` and `test/main/search-test` passed on C and Rust: 2 + 8 reached assertions on each binary. Before this slice, Rust skipped `search-preload-test` with zero reached assertions. The focused strict runner reported `PASS_SELECTED_SCRIPTS`; the 154-script gate belongs to the next integrated main checkpoint.

Checks: `cargo fmt --check`; `cargo test --locked --quiet` (92 + 24 + 2); `cargo clippy --locked --all-targets -- -D warnings`; `cargo build --release --locked --quiet`; `prompt-history-pty.py`; `prompt-completion-pty.py` (actions, set/toggle, variables, relative and quoted paths, live `:source`, ambiguous matches, control-byte display). Two independent read-only reviews found and prompted fixes for empty relative parents, path quoting, apostrophes, and match-list terminal escaping.

The version string claims **readline-style history and completion**, not GNU Readline linkage or full line-editing parity. Completion offers only Rust-supported command variables; upstream `file-old` and `lineno-old` expansion remains outside this slice.

SHA-256:

- `rust/lib.rs`: `77430bce9ced48d3e6d15e94d2daeaf7055e74b6e41e64616cfb62f426e1446d`
- `rust/main.rs`: `595503ce83a17194768427c6070538c0a394d3e27a98ddfa9f36b2cabfe17ae4`
- `rust/options_catalog.rs`: `8f070a0695a25a17329c29b002ff820c79eda73c6eab57ade9547ae623d86f5f`
- `rust/prompt.rs`: `aa9878efbdb6135469e3e5593d028d55ad5ed66ce0c8baa269c282e076595bc5`
- `rust/tests/prompt-completion-pty.py`: `f91f77bd781f48c454afb9e6b4069b2e3a0596538d365f701a086e207d210b0c`
- `target/release/tig`: `f321e3b35da1b26067bc6664ef3463bfac3ba8d977e780d131cfcb83cf0be834`
