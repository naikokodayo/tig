# Tig Rust migration rules

The goal is a safe, idiomatic Rust Tig with the original user behavior. Keep
first-party Rust free of `unsafe`; preserve GPL-2.0-or-later notices and credit
the upstream Tig project. The C source and original tests remain the reference
until the Rust parity gate is closed. Never change an original assertion to
make a Rust result pass.

## Change one behavior chain at a time

- Trace the affected callers before editing. Reuse an existing helper or a
  maintained dependency before adding another implementation.
- Give each PR a small, named set of original scripts and a concrete C/Rust
  before-and-after result. Keep unrelated `rust/main.rs` changes in separate
  integration slices; avoid concurrent edits to `MIGRATION.md` when possible.
- When replacing Rust behavior, remove the superseded branch, helper, test, or
  dependency in the same PR. Search all callers first. Do not keep unused
  compatibility layers or duplicate render and refresh paths.
- Preserve lossless paths, checked Git index updates, terminal restoration,
  command argument boundaries, and fail-closed behavior for unsupported input.

## Verify and integrate

- Run formatting, compilation, relevant unit tests, and focused original
  C/Rust scripts locally. Use real Git index and terminal regressions for
  changes to staging, commands, paths, or screen layout.
- Keep the 154-script strict runner fail-closed. Run the complete paired suite
  on integrated main checkpoints; avoid repeating it for every branch edit.
  Write its current result to `migration/evidence/upstream-rust-only-current.json`
  and replace that file at the next checkpoint; Git preserves older results.
  Distinguish a passing assertion from a skipped or unreached one.
- Keep one final, source-and-binary-hashed receipt per slice and the raw
  failure data needed to explain exceptions. Do not multiply near-identical
  review logs. Historical receipts remain available in Git history.
- Recheck the combined source against the latest main before merging. A
  passing feature branch that does not compile after merge is unfinished.
  Independent reviewers are read-only; merge a clean PR promptly.

The end-to-end C/Rust application benchmark belongs after the parity gate.

## Migration kit

Use the full [Codex Migration Kit skill](https://github.com/naikokodayo/code-migration-kit-with-codex/tree/main/.agents/skills/code-migration) when planning migration batches. Follow its **redesign** path: the work unit is a behavior or subsystem, not a one-to-one C file translation. The current phase is public C/Rust behavior matching; the strict original-script adapter is the judge. The reviewed C dependency map and Rust boundary decisions are in `migration/depmap/` and `migration/ARCHITECTURE.md`. Keep the kit's rulebook/design decisions read-only while implementers are working. Do not install the kit's restrictive local execution rules on someone else's behalf.
