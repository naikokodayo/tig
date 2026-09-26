# Split-chunk display parity

`stage_split_chunk` in `src/stage.c` rewrites view rows through
`stage_insert_chunk`; it does not apply a patch or write the index. Its scan
stops at the first no-newline marker. In the unchanged original
`test/stage/split-chunk-test`, that leaves the last displayed header at
`@@ -8,4 +8,1 @@`, while the complete valid patch has new count 2.
This is a display discrepancy, not evidence of C data loss.

Rust now returns both valid patch bytes and matching display rows from the
existing `Patch::split_hunk` operation. The application replaces the same row
range in each representation. Later staging still parses `view.raw_patch`,
never the display header; repeated splits preserve other display headers.
No second diff parser, unsafe code, dependency, or original assertion change
was introduced.

The existing real-Git unit regression checks the exact split display headers,
byte-identical index/worktree immediately after splitting, reparses the valid
raw split patch, and stages its four hunks to reproduce the working file.
`rust/tests/split-chunk.py` additionally exercises the public C/Rust executable
in a controlling PTY, with staged/unstaged and newline/no-newline fixtures,
checking all headers and unchanged actual index entries/worktree bytes.

The concise final receipt is `evidence/split-chunk-focused.json`: the unchanged
original split-chunk script before/after, the eight public-PTY safety cases,
and source/binary hashes. It retains the baseline split-header failure only;
passing raw transcripts and unrelated neighboring runs are omitted. C passes
both original assertions before and after; Rust improves from one passing
assertion to both. All eight safety cases pass. This scoped receipt does not
claim that the full 154-script parity gate has closed.
