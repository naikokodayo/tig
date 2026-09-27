#!/usr/bin/env python3
"""Exercise history preload and prompt navigation through a controlling PTY."""
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


def run(binary, repo, home, script=None, keys=b""):
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 30, 80, 0, 0))

    def setup():
        os.setsid()
        fcntl.ioctl(0, termios.TIOCSCTTY, 0)

    env = {**os.environ, "HOME": str(home), "TERM": "xterm", "XDG_DATA_HOME": ""}
    if script:
        env["TIG_SCRIPT"] = str(script)
    else:
        env.pop("TIG_SCRIPT", None)
    process = subprocess.Popen([str(binary)], cwd=repo, env=env, stdin=slave,
                               stdout=slave, stderr=slave, preexec_fn=setup)
    os.close(slave)
    output = bytearray()
    try:
        if keys:
            time.sleep(0.4)
            os.write(master, keys)
        deadline = time.monotonic() + 10
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


def main():
    binary = Path(sys.argv[1] if len(sys.argv) > 1 else "target/release/tig").resolve()
    with tempfile.TemporaryDirectory() as temp:
        root = Path(temp)
        repo = root / "repo"
        repo.mkdir()
        subprocess.run(["git", "init", "-q", str(repo)], check=True)
        for message in ("initial", "needle", "latest"):
            (repo / "file").write_text(message)
            subprocess.run(["git", "add", "file"], cwd=repo, check=True)
            subprocess.run(["git", "-c", "user.name=Test", "-c", "user.email=test@example.com",
                            "commit", "-qm", message], cwd=repo, check=True)
        home = root / "home"
        home.mkdir()
        history = home / ".tig_history"
        history.write_text("needle\n")
        display = root / "display"
        script = root / "script"
        script.write_text(f":find-next\n:save-display {display}\n")
        run(binary, repo, home, script=script)
        screen = display.read_text()
        assert "commit 2 of 3" in screen and "needle" in screen, screen
        run(binary, repo, home, keys=b"/\x1b[Ax\rQ")
        assert history.read_text().splitlines() == ["needle", "needlex"], history.read_text()


if __name__ == "__main__":
    main()
