# Rust stash patch handoff

Implementation commit: `d2b72a1cadb9c98f8d20d6b4c209d02c2216b80d`.
Rust release `tig` SHA-256: `18c372ea6ecbfbf69eaebfa17cbd9b70bfe2c98ef36a588fdb555aa14c1056c6`.

Opening a stash row now uses Git's `stash show --patch-with-stat` directly,
including Git's configured untracked-file behavior. It no longer reconstructs
the patch from the stash commit's first parent. The existing diff option
validation and no-textconv/no-ext-diff boundary remain in force.

Rust 1.81 formatting, 114 unit tests, Clippy with warnings denied, and release
build passed. `rust/tests/reflog-stash-navigation.py` verified older stash
selection, diff opening, closing, and refresh, plus C/Rust display of an
untracked file included by `stash.showIncludeUntracked`. The unchanged
`test/stash/start-on-line-test` passed on both implementations (1 assertion
each). The full 154-script stage gate remains open.
