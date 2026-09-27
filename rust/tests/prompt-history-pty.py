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


def run(binary, repo, home, script=None, keys=b"", columns=80):
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 30, columns, 0, 0))

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
    return bytes(output)


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
        history.write_text("old\nneedle\nneedle\n")
        (home / ".tigrc").write_text("set history-size = 2\n")
        display = root / "display"
        script = root / "script"
        script.write_text(f":find-next\n:save-display {display}\n")
        run(binary, repo, home, script=script)
        screen = display.read_text()
        assert "commit 2 of 3" in screen and "needle" in screen, screen
        assert history.read_text().splitlines() == ["needle"], history.read_text()
        run(binary, repo, home, keys=b"/\x1b[Ax\rQ")
        assert history.read_text().splitlines() == ["needle", "needlex"], history.read_text()
        damaged = b"valid\n\xff\n"
        history.write_bytes(damaged)
        run(binary, repo, home, keys=b"Q")
        assert history.read_bytes() == damaged
        protected = b"a\nb\n"
        history.write_bytes(protected)
        history.chmod(0o400)
        run(binary, repo, home, keys=b"Q")
        assert history.read_bytes() == protected and history.stat().st_mode & 0o777 == 0o400
        history.unlink()
        target = root / "symlink-target"
        target.write_bytes(protected)
        history.symlink_to(target)
        run(binary, repo, home, keys=b"Q")
        assert history.is_symlink() and target.read_bytes() == protected
        history.unlink()
        run(binary, repo, home, keys=b"/needle\rQ")
        assert history.read_text() == "needle\n"
        run(binary, repo, home, keys="/ab\x1b[D\u00e9\x01Z\x05Y\x1b[D\x1b[D\x7f\rQ".encode())
        assert history.read_text().splitlines()[-1] == "ZabY"
        run(binary, repo, home, keys=b"/abc\x01\x1b[C\x1b[3~\rQ")
        assert history.read_text().splitlines()[-1] == "ac"
        for grapheme in ("👩‍🔬", "👋🏽"):
            screen = run(binary, repo, home,
                         keys=f"/12345{grapheme}\rQQQ".encode(), columns=8).decode(errors="replace")
            assert f"\x1b[2K12345{grapheme}\x1b[30;8H" in screen
        run(binary, repo, home, keys="/A👩‍🔬B\x1b[D\x7f\rQQQ".encode())
        assert history.read_text().splitlines()[-1] == "AB"
        run(binary, repo, home, keys="/A👋🏽B\x01\x1b[C\x1b[3~\rQQQ".encode())
        assert history.read_text().splitlines()[-1] == "AB"
        screen = run(binary, repo, home,
                     keys="/123456e\u0301\x7f\rQQQ".encode(), columns=8).decode(errors="replace")
        assert "\x1b[2K123456e\u0301\x1b[30;8H" in screen
        assert history.read_text().splitlines()[-1] == "123456"
        (home / ".inputrc").write_text(
            '$if other\n"\\C-a": beginning-of-line\n$endif\n'
            '$if tig\n"\\C-a": end-of-line\n$endif\n'
        )
        run(binary, repo, home, keys=b"/ab\x01X\rQQQ")
        assert history.read_text().splitlines()[-1] == "abX"
        (home / ".inputrc").write_text('$if tig\n"\\C-e": beginning-of-line\n$endif\n')
        run(binary, repo, home, keys=b"/ab\x05X\rQQQ")
        assert history.read_text().splitlines()[-1] == "Xab"
        saved_history = history.read_bytes()
        (home / ".inputrc").write_text('$if tig\n"\\C-c": end-of-line\n$endif\n')
        run(binary, repo, home, keys=b"/ab\x03X\rQQQQQ")
        assert history.read_bytes() == saved_history
        run(binary, repo, home, keys=b":tog\t author\rQQQ")
        assert history.read_text().splitlines()[-1] == "toggle author"
        run(binary, repo, home, keys=b":ech\t hello\rQQQ")
        assert history.read_text().splitlines()[-1] == "echo hello"
        run(binary, repo, home, keys=b":refr\t\rQQQ")
        assert history.read_text().splitlines()[-1] == "refresh"
        run(binary, repo, home, keys=b":e\t\rQQQ")
        assert history.read_text().splitlines()[-1] == "e"


if __name__ == "__main__":
    main()
