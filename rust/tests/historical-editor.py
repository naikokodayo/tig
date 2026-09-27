#!/usr/bin/env python3
"""Check historical edit targets against real Git and a controlling terminal."""
import fcntl
import os
from pathlib import Path
import pty
import select
import struct
import subprocess
import sys
import tempfile
import termios
import time


binary = Path(sys.argv[1]).resolve()


def git(repo, *args):
    return subprocess.check_output(["git", "-C", str(repo), *args])


def run(repo, script, editor, capture):
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 24, 80, 0, 0))

    def terminal():
        os.setsid()
        fcntl.ioctl(0, termios.TIOCSCTTY, 0)

    env = dict(os.environ, TIG_SCRIPT=str(script), TIG_EDITOR=str(editor),
               CAPTURE=str(capture), TIGRC_SYSTEM=str(Path(__file__).resolve().parents[2] / "tigrc"),
               TIGRC_USER="/dev/null", GIT_CONFIG_NOSYSTEM="1", GIT_CONFIG_GLOBAL="/dev/null")
    process = subprocess.Popen([str(binary), "-C", str(repo)], env=env,
                               stdin=slave, stdout=slave, stderr=slave, preexec_fn=terminal)
    os.close(slave)
    output = bytearray()
    deadline = time.monotonic() + 10
    try:
        while process.poll() is None and time.monotonic() < deadline:
            if select.select([master], [], [], .1)[0]:
                try:
                    output.extend(os.read(master, 65536))
                except OSError:
                    break
        assert process.wait(timeout=1) == 0, output.decode(errors="replace")
    finally:
        if process.poll() is None:
            process.kill()
            process.wait()
        os.close(master)


with tempfile.TemporaryDirectory() as directory:
    root = Path(directory)
    repo = root / "repo"
    repo.mkdir()
    git(repo, "init", "-q")
    git(repo, "config", "user.name", "Test")
    git(repo, "config", "user.email", "test@example.invalid")
    worktree = repo / "file.rs"
    worktree.write_bytes(b"old version\n")
    git(repo, "add", "file.rs")
    git(repo, "commit", "-qm", "old")
    worktree.write_bytes(b"new version\n")
    git(repo, "commit", "-qam", "new")
    index = git(repo, "ls-files", "--stage", "-z")
    editor = root / "editor"
    editor.write_text('#!/bin/sh\nfor file do :; done\ncp "$file" "$CAPTURE"\nprintf "%s" "$file" > "$CAPTURE.path"\n')
    editor.chmod(0o700)
    script = root / "script"
    capture = root / "capture"
    for actions in (":goto 2\n:view-tree\n:edit\n", ":goto 2\n:view-tree\n:enter\n:edit\n"):
        script.write_text(actions + ":quit\n")
        run(repo, script, editor, capture)
        assert capture.read_bytes() == b"old version\n"
        temp = Path(str(capture) + ".path").read_text()
        assert temp != "file.rs" and not Path(temp).exists(), temp
        assert worktree.read_bytes() == b"new version\n"
        assert git(repo, "ls-files", "--stage", "-z") == index
    script.write_text(":goto 1\n:view-tree\n:edit\n:quit\n")
    run(repo, script, editor, capture)
    assert capture.read_bytes() == b"new version\n"
    assert Path(str(capture) + ".path").read_text() == "file.rs"
    worktree.write_bytes(b"private worktree edits\n")
    script.write_text(":goto 1\n:view-tree\n:enter\n:edit\n:quit\n")
    run(repo, script, editor, capture)
    assert capture.read_bytes() == b"new version\n"
    temp = Path(str(capture) + ".path").read_text()
    assert temp != "file.rs" and not Path(temp).exists(), temp
    assert worktree.read_bytes() == b"private worktree edits\n"
    assert git(repo, "ls-files", "--stage", "-z") == index
    bare = root / "bare.git"
    subprocess.run(["git", "clone", "-q", "--bare", str(repo), str(bare)], check=True)
    script.write_text(":goto 1\n:view-tree\n:enter\n:edit\n:quit\n")
    run(bare, script, editor, capture)
    assert capture.read_bytes() == b"new version\n"
    temp = Path(str(capture) + ".path").read_text()
    assert temp != "file.rs" and not Path(temp).exists(), temp
    filtered = root / "filtered"
    filtered.mkdir()
    git(filtered, "init", "-q")
    git(filtered, "config", "user.name", "Test")
    git(filtered, "config", "user.email", "test@example.invalid")
    (filtered / ".gitattributes").write_text("file text eol=crlf\n")
    (filtered / "file").write_bytes(b"line\n")
    git(filtered, "add", ".gitattributes", "file")
    git(filtered, "commit", "-qm", "filtered")
    (filtered / "file").unlink()
    git(filtered, "checkout", "-f", "HEAD", "--", "file")
    assert (filtered / "file").read_bytes() == b"line\r\n"
    assert not git(filtered, "status", "--short")
    script.write_text(":goto 1\n:view-tree\n:goto 3\n:enter\n:edit\n:quit\n")
    run(filtered, script, editor, capture)
    assert capture.read_bytes() == b"line\r\n"
    assert Path(str(capture) + ".path").read_text() == "file"
print("historical/dirty/bare snapshots and clean filtered worktree editor: OK")
