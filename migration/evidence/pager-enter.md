# Pager Enter behavior

Base: merged `main` at `eba0d6da`. Rust pager text now recognizes full or abbreviated hexadecimal commit headers. Enter opens that commit's diff; Enter on other pager lines scrolls one line. With `focus-child = no`, the opened diff stays beside the focused pager and the pager advances, matching C. Log Enter remains unchanged: body rows still open their owning commit's diff.

Verification: 25 Rust application tests, formatting, Clippy with warnings denied, and release build pass on the combined source. Three unchanged original scripts (`test/log/diff-stat-test`, `test/log/pretty-format-test`, `test/main/show-changes-test`) pass on C and Rust, with 15 reached assertions per side. Focused unit checks cover plain pager scrolling, commit header classification, and real-Git `focus-child = no` opening/scrolling. Independent read-only review found no remaining material issue. The complete 154-script gate remains the previous integrated checkpoint.

SHA-256: `rust/main.rs` `77633e3fc106764107f80923bf20e06c56aa759ef9adc0204843061fb816365c`; `target/release/tig` `81f012f2d1b76d9ea452cedf3bc4a6aa13587bc7bd7a3782d51bc8c13b67a999`.
