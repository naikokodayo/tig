# Literal leading-dash grep pattern receipt

Base: `11378de61943dff7531299f060fbe3a4d6f86596` (`origin/main`). Rust now forwards the first `--` as Git's option terminator and reads the following word, including `-foo` or `--`, as the literal pattern. Later revision operands and a second `--` path separator retain their roles. A lone first `--` still fails for a missing pattern, and an option after the pattern without a path separator remains an error. No unsafe code or new Python was added.

| Source or binary | SHA-256 |
| --- | --- |
| `rust/grep.rs` | `835e77161ee3a9b1c28e8050bddefaa84bc84a9c86eebb8e8a8ee76825f51b77` |
| `test/grep/default-test` | `b1851b78638060c7bf6456d1e135346155e3b1b8ccc6061870de1d95b09a3acc` |
| `test/grep/refspec-test` | `b6157d8b16a2f3b3b45d89a5b148b28d28518ece49d640f1692e67253623b455` |
| `src/tig` (C) | `54db05e55837e0e9ea34d91b08957faca549285e437683cd31fc504f222b6dd1` |
| `target/release/tig` (Rust) | `fac94094f87c10ca31e376e978dad0aefa56d8e24563d035e734b588c66ad30a` |

Checks: `cargo fmt --check`, `cargo test` (125 tests), `cargo clippy --all-targets -- -D warnings`, and `cargo build --release` passed; the focused parser test passed again after rebasing. Original `test/grep/default-test` passed 9/9 assertions and `test/grep/refspec-test` 2/2 in both C and Rust, unchanged. Existing `grep-revisions.py` passed 36 checks. A disposable real Git/PTY C/Rust comparison passed `-- -foo`, `-- -foo HEAD`, `-- -foo HEAD -- file`, and `-e -foo -- file` against a committed `-foo xx -foo` line. The comparison normalized only the pre-existing C `x` versus Rust `|` line-number separator. That probe stayed outside the repository.
