# C core dependency map

Generated with the [Codex Migration Kit](https://github.com/naikokodayo/code-migration-kit-with-codex/tree/da426c4/.agents/skills/code-migration) C mapper at `da426c4`, using tracked `src/` and `include/tig/` from Tig. Their Git tree IDs are `92180143f33365a90c1994b11840895272f0a051` and `0be3156b781b63ac5104a006bf0e32910946a9c6`. The command uses `--include-dir include --own-header-dir include/tig` so `-Iinclude` headers and separate same-stem headers are included.

The map contains **77 files, 367 edges, 76 batches, and one cycle** (`include/tig/string.h` ↔ `include/tig/tig.h`). Two independent read-only reviews inspected disjoint source samples. The first 40 files found three missing same-stem edges; the mapper was fixed. A fresh review of the other 37 files found no missing or invented edges, and the first three misses are present in the final map.

This map covers the tracked C core, not `compat/` or generated files. For example, the `compat/hashtab.h` include is outside this map. It informs subsystem boundaries; it is **not** a one-C-file-to-one-Rust-file work queue. Tig's Rust migration follows the kit's redesign path and the public C/Rust parity judge.
