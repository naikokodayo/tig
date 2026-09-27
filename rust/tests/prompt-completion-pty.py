#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-2.0-or-later
"""Check interactive prompt completion through the real terminal."""
import fcntl
import os
import pathlib
import pty
import select
import struct
import subprocess
import sys
import tempfile
import termios
import time


def run(binary, repo, home, keys):
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 30, 80, 0, 0))

    def setup():
        os.setsid()
        fcntl.ioctl(0, termios.TIOCSCTTY, 0)

    env = {**os.environ, "HOME": str(home), "TERM": "xterm", "XDG_DATA_HOME": ""}
    process = subprocess.Popen([str(binary)], cwd=repo, env=env, stdin=slave,
                               stdout=slave, stderr=slave, preexec_fn=setup)
    os.close(slave)
    output = bytearray()
    deadline = time.monotonic() + 10
    try:
        while b"[main]" not in output and time.monotonic() < deadline:
            if select.select([master], [], [], 0.1)[0]:
                output.extend(os.read(master, 65536))
        assert b"[main]" in output, output.decode(errors="replace")
        os.write(master, keys)
        while process.poll() is None and time.monotonic() < deadline:
            if select.select([master], [], [], 0.1)[0]:
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
    return bytes(output)


def main():
    binary = pathlib.Path(sys.argv[1]).resolve()
    with tempfile.TemporaryDirectory() as temp:
        root = pathlib.Path(temp)
        repo = root / "repo"
        repo.mkdir()
        subprocess.run(["git", "init", "-q", str(repo)], check=True)
        (repo / "file").write_text("content")
        subprocess.run(["git", "add", "file"], cwd=repo, check=True)
        subprocess.run(["git", "-c", "user.name=Test", "-c", "user.email=test@example.com",
                        "commit", "-qm", "initial"], cwd=repo, check=True)
        home = root / "home"
        home.mkdir()
        history = home / ".tig_history"
        (repo / "prompt-settings.tigrc").write_text("set line-graphics = ascii\n")
        (repo / "space settings.tigrc").write_text("bind generic Z :echo loaded\n")
        (repo / "escape-\x1b[31m").write_text("")
        (repo / "escape-other").write_text("")
        cases = [
            (b":tog\t wrap-search\rQQQ", "toggle wrap-search"),
            (b":set history-s\t\rQQQ", "set history-size = "),
            (b":toggle wrap-s\t\rQQQ", "toggle wrap-search"),
            (b":source prompt-s\t\rQQQ", "source prompt-settings.tigrc"),
            (b":echo %(repo:head-i\t\rQQQ", "echo %(repo:head-id)"),
        ]
        for keys, expected in cases:
            try:
                run(binary, repo, home, keys=keys)
            except Exception as error:
                raise AssertionError(f"prompt keys {keys!r} failed") from error
            assert history.read_text().splitlines()[-1] == expected, (keys, history.read_text())
        screen = run(binary, repo, home, keys=b":source spa\t\rZQQQ")
        assert history.read_text().splitlines()[-1] == 'source "space settings.tigrc"'
        assert b"loaded" in screen, screen.decode(errors="replace")
        screen = run(binary, repo, home, keys=b":set diff-\t\tX\rQQQ")
        assert b"matches: diff-context" in screen, screen.decode(errors="replace")
        screen = run(binary, repo, home, keys=b":source escape-\t\tX\rQQQ")
        assert b"escape-\\x1b[31m" in screen, screen.decode(errors="replace")


if __name__ == "__main__":
    main()
