#!/usr/bin/env python3
"""Check historical edit targets against real Git and a controlling terminal."""
import errno
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
    # The original file-name screen expects C's quoted-path truncation. These
    # checks independently verify the displayed name and the real editor/blob.
    names = root / "names"
    names.mkdir()
    git(names, "init", "-q")
    git(names, "config", "user.name", "Path Fixture")
    git(names, "config", "user.email", "paths@example.invalid")
    folder = names / "-- foo bar"
    folder.mkdir()
    files = [b"-- boo far", "as测试asd".encode()]
    try:
        fd = os.open(os.fsencode(folder) + b"/raw-\xff", os.O_WRONLY | os.O_CREAT, 0o600)
        os.close(fd)
        files.append(b"raw-\xff")
    except OSError as error:
        if error.errno not in (errno.EILSEQ, errno.EINVAL, errno.ENOTSUP):
            raise
        print(f"non-UTF-8 worktree filename unavailable: {error}")
    for i, name in enumerate(files):
        with open(os.fsencode(folder) + b"/" + name, "wb") as file:
            file.write(f"path payload {i}\n".encode())
    git(names, "add", "--", "-- foo bar")
    git(names, "commit", "-qm", "path fixture")
    index = git(names, "ls-files", "--stage", "-z")
    screen = root / "names.screen"
    blob = root / "blob.screen"
    root_screen = root / "root.screen"
    for i, name in enumerate(files):
        for open_blob in (False, True):
            script.write_text(f":view-tree\n:save-display {root_screen}\n:enter\n"
                              + f":goto {i + 3}\n"
                              + f":save-display {screen}\n"
                              + (f":enter\n:save-display {blob}\n" if open_blob else "")
                              + ":edit\n:quit\n")
            run(names, script, editor, capture)
            directory_row = root_screen.read_text().splitlines()[1]
            assert directory_row.endswith("-- foo bar") and "Path Fixture" in directory_row, directory_row
            display = screen.read_text()
            row = next(line for line in display.splitlines()
                       if line.endswith(name.decode(errors="replace")))
            assert "Path Fixture" in row, row
            assert "Path Fixture" in display.splitlines()[2], display
            expected = f"path payload {i}\n".encode()
            assert capture.read_bytes() == expected
            assert Path(str(capture) + ".path").read_bytes() == b"./-- foo bar/" + name
            if open_blob:
                assert expected.decode().strip() in blob.read_text()
            assert git(names, "ls-files", "--stage", "-z") == index
            with open(os.fsencode(folder) + b"/" + name, "rb") as file:
                assert file.read() == expected
    print(f"filename display/metadata/blob/editor: {len(files) * 2} checks passed")
print("historical/dirty/bare snapshots and clean filtered worktree editor: OK")
