# Blame context navigation receipt

Base: `01f54d60` (latest main before PR); baseline failures reproduced on unmodified `40187436`. Safe Rust, GPL attribution, original assertions and dependencies unchanged.

Before (baseline Rust, real Git/PTY): rename blob failed with `No selected blob`; main selected the first synthetic changes row instead of the attributed commit; null Enter failed resolving the zero OID. After: historical filename/revision drives blob; main removes inherited file/revision filters and selects attribution; null Enter opens only that file’s worktree diff. Close restores blame, and failed opens restore its context. Null blob follows C’s HEAD fallback.

Validation after rebase (macOS arm64):
- `cargo fmt --check`, `cargo test --locked` (125 tests), `cargo clippy --locked --all-targets -- -D warnings`, `cargo build --locked --release`, `git diff --check`: pass.
- `python3 rust/tests/blame-context.py`: 12 executions checked: 11 functional navigation passes and one asserted C no-parent failure baseline. Rename blob/main, inherited filters, dirty-line HEAD blob, null diff with/without parent, refresh/close, index/worktree unchanged. No specific hunk selection required.
- `python3 rust/tests/file-blame.py`: 31 checks pass; `python3 rust/tests/blame-navigation.py`: 6 checks pass.
- `python3 rust/tests/upstream-suite.py test/blame/default-test test/blame/blob-blame-test test/blame/navigation-parent-test test/blame/revargs-test test/blame/initial-diff-test test/main/goto-test test/main/filter-args-test test/blob/wrap-lines-test --output ../focused.json`: C 8 scripts / 22 assertions pass; Rust 7 scripts pass, 1 fails (20 assertions pass, 2 fail), no skips. No full 154-script run. Rust failure matches the unmodified baseline receipt byte-for-byte: existing internal command traces in `filter-args-test`, whose screen assertion passes. Strict runner remains BLOCKED; no dummy Git calls added.

C exceptions observed and asserted: null diff refresh discards its blame command and displays HEAD; Rust retains the file’s working diff. For a staged new file with no parent, C displays an unknown `encoding=UTF-8` option error from Git’s no-index diff; Rust displays the added file. These are explicit differences, not C/Rust parity passes. External diff/textconv remain disabled.

Final independent read-only acceptance review: no remaining findings. Earlier findings (HEAD fallback and failed-open context restoration) were fixed and checked.

SHA-256 (base plus source hashes identifies tested code):
- `rust/main.rs`: `63ff5727388e11cb7f228e718825ea5856998fdc5ead0c635a575ad869e78408`
- `rust/git.rs`: `9d53b2e51ed0a512447b4b457c7a44949c72618959273234aafb2eb3847f8945`
- `rust/tests/blame-context.py`: `1f93fb111c2b7c18905609a2400f49f952e30d1679cfc4d0359d1fb47f0f316d`
- `target/release/tig`: `28fec1eea2e641782b86610256ea28e2b82babfcc07946c5f71275c9860c0c8a`
- `src/tig`: `d3724c51f790554ed07c50cfc6d99342122f53585c3ab4a048fa1a408fc95b7d`

Baseline Rust binary SHA-256: `d5dd522d9addac44017141c73d09ddf9175160571da3b8969a0825c47b70fbc1`.

Original failure output (ANSI styling removed only; identical before/after):

```diff
[FAIL] rev-parse.trace != expected/rev-parse.trace
diff --git a/expected/rev-parse.trace b/rev-parse.trace
index c6d0c82..aba1ee8 100644
--- a/expected/rev-parse.trace
+++ b/rev-parse.trace
@@ -1,4 +1,5 @@
-git rev-parse --no-revs --no-flags -- common tracer
-git rev-parse --flags --no-revs -- common tracer
-git rev-parse --symbolic --revs-only -- common tracer
-git rev-parse --git-dir --is-inside-work-tree --show-cdup --show-prefix HEAD --symbolic-full-name HEAD
+git rev-parse --absolute-git-dir 
+git rev-parse --is-bare-repository 
+git rev-parse --show-toplevel 
+git rev-parse --symbolic-full-name @{upstream} 
+git rev-parse --verify --end-of-options HEAD^{commit} 
[FAIL] log.trace != expected/log.trace
diff --git a/expected/log.trace b/log.trace
index a9795ce..d173664 100644
--- a/expected/log.trace
+++ b/log.trace
@@ -1 +1 @@
-git log --encoding=UTF-8 --topo-order --exclude=refs/remotes/origin/* --exclude=refs/heads/master --all --date=raw --parents --no-color --show-notes --pretty=format:commit %m %H %P%x00%aN <%aE> %ad%x00%cN <%cE> %cd%x00%s%x00%N%x03 -- common tracer
+git log --parents --no-show-signature --decorate=full --format=%m%H%x00%P%x00%aN%x00%aI%x00%s%x00%D%x00%aE%x00%cN%x00%cE%x00%cI%x00%N -z --show-notes --topo-order --exclude=refs/remotes/origin/* --exclude=refs/heads/master --all -- common tracer 
  [OK] filtered.screen assertion
```
