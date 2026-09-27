# Rust stash patch handoff

Implementation commit: `65fbaf7a090d630b2cd3c6f974451ed9db54cb32`.
Rust release `tig` SHA-256: `7352e633f968702fc9ec7ccefa8efc302a60a16dff096249204c6a1d67441bce`.

Opening a stash row now uses Git's `stash show --patch-with-stat` directly,
including Git's configured untracked-file behavior. It no longer reconstructs
the patch from the stash commit's first parent. The existing diff option
validation, configured whitespace handling, and no-textconv/no-ext-diff
boundary remain in force. The stash patch source survives diff refresh.

Rust 1.81 formatting, 114 unit tests, Clippy with warnings denied, and release
build passed. `rust/tests/reflog-stash-navigation.py` verified older stash
selection, diff opening, closing, and refresh, plus C/Rust initial display of
an untracked file included by `stash.showIncludeUntracked` and omission of a
whitespace-only hunk. Rust retains this content after refreshing the child
diff; upstream C falls back to a generic merge-commit view on that refresh.
The unchanged
`test/stash/start-on-line-test` passed on both implementations (1 assertion
each). The full 154-script stage gate remains open.
