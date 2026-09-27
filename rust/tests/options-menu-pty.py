#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-2.0-or-later
"""Compare real menu selections with existing toggle commands (C or Rust)."""
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
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 30, 100, 0, 0))
    def setup():
        os.setsid()
        fcntl.ioctl(0, termios.TIOCSCTTY, 0)
    env = {**os.environ, "HOME": str(home), "TERM": "xterm", "TIGRC_SYSTEM": str(pathlib.Path(__file__).resolve().parents[2] / "tigrc"),
           "TIGRC_USER": str(home / ".tigrc"), "XDG_DATA_HOME": ""}
    env.pop("TIG_SCRIPT", None)
    process = subprocess.Popen([str(binary)], cwd=repo, env=env, stdin=slave,
                               stdout=slave, stderr=slave, preexec_fn=setup)
    os.close(slave)
    output = bytearray()
    def drain(seconds):
        end = time.monotonic() + seconds
        while time.monotonic() < end:
            if select.select([master], [], [], 0.02)[0]:
                try:
                    data = os.read(master, 65536)
                except OSError:
                    break
                if not data:
                    break
                output.extend(data)
    try:
        deadline = time.monotonic() + 5
        while b"[main]" not in output and time.monotonic() < deadline:
            drain(0.1)
        assert b"[main]" in output, output
        for chunk in keys:
            os.write(master, chunk)
            drain(0.15)
        assert process.wait(timeout=3) == 0, output
    finally:
        if process.poll() is None:
            process.kill()
            process.wait()
        os.close(master)
    return bytes(output)

binary = pathlib.Path(sys.argv[1]).resolve()
with tempfile.TemporaryDirectory() as temp:
    root = pathlib.Path(temp)
    repo, home = root / "repo", root / "home"
    repo.mkdir()
    home.mkdir()
    subprocess.run(["git", "init", "-q", str(repo)], check=True)
    (repo / "file").write_text("content\n")
    subprocess.run(["git", "add", "file"], cwd=repo, check=True)
    subprocess.run(["git", "-c", "user.name=Test", "-c", "user.email=test@example.com",
                    "commit", "-qm", "initial"], cwd=repo, check=True)
    # Isolate configuration and make column/global outcomes observable.
    (home / ".tigrc").write_text("set show-changes = no\n")

    def capture(keys, name):
        chunks = [keys]
        for prefix in (b"o", b":options\r"):
            if keys.startswith(prefix):
                chunks = [prefix, keys[len(prefix):]]
                break
        screen = run(binary, repo, home, [*chunks, f":save-options {name}.rc\r".encode(), b"Q"])
        return screen, (repo / f"{name}.rc").read_text()

    cases = [
        (b"o\r", "line-number"),
        (b":options\rA", "author"),
        (b"o\x1bOB\r", "date"),
        (b"o\x1bOC\r", "date"),
        (b"o\x1bOA\r", "vertical-split"),
        (b"o\x1bOD\r", "vertical-split"),
        (b"o\x1bOD\x1bOC\r", "line-number"),
        (b"o?W", "ignore-space"),
        (b"o\x03", None),
        (b"o\x1b", None),
    ]
    for i, (keys, option) in enumerate(cases):
        expected = capture(f":toggle {option}\r".encode() if option else b"", f"direct{i}")
        actual = capture(keys, f"menu{i}")
        assert b"Toggle option line numbers" in actual[0], (i, actual[0])
        assert actual[1:] == expected[1:], (i, option, "saved settings differ")
    print(f"PASS: {len(cases)} options menu terminal cases ({binary})")
