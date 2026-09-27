#!/usr/bin/env python3
"""Real-terminal status revert regressions. Usage: status-revert.py BINARY [--c]."""
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

BINARY = str(Path(sys.argv[1]).resolve())
IS_C = '--c' in sys.argv

def git(root, *args, ok=True):
    result = subprocess.run(['git', '-C', str(root), *args], capture_output=True)
    assert not ok or result.returncode == 0, result.stderr
    return result.stdout

class Terminal:
    def __init__(self, root):
        self.master, slave = pty.openpty()
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 30, 140, 0, 0))
        def tty():
            os.setsid()
            fcntl.ioctl(0, termios.TIOCSCTTY, 0)
        env = dict(os.environ, TERM='xterm-256color', TIGRC_SYSTEM=str(Path(__file__).resolve().parents[2] / 'tigrc'),
                   TIGRC_USER='/dev/null', GIT_CONFIG_NOSYSTEM='1', GIT_CONFIG_GLOBAL='/dev/null')
        env.pop('TIG_SCRIPT', None)
        self.process = subprocess.Popen([BINARY, '-C', str(root), 'status'], env=env,
            stdin=slave, stdout=slave, stderr=slave, preexec_fn=tty)
        os.close(slave)
        self.output = b''
        deadline = time.monotonic() + 5
        while b'[status]' not in self.output and time.monotonic() < deadline:
            self.drain()
        assert b'[status]' in self.output, self.output
    def drain(self):
        end = time.monotonic() + .35
        while time.monotonic() < end:
            if select.select([self.master], [], [], .03)[0]:
                try:
                    self.output += os.read(self.master, 65536)
                except OSError:
                    break
    def send(self, keys):
        os.write(self.master, keys)
        self.drain()
    def close(self):
        self.send(b'Q')
        try:
            self.process.wait(timeout=3)
        except subprocess.TimeoutExpired:
            self.process.kill()
            raise AssertionError(self.output.decode(errors='replace'))
        finally:
            os.close(self.master)
        assert self.process.returncode == 0, self.output

def fixture(root):
    git(root, 'init', '-q')
    git(root, 'config', 'user.name', 'Test')
    git(root, 'config', 'user.email', 'test@example.invalid')
    git(root, 'config', 'commit.gpgsign', 'false')
    (root / 'file').write_bytes(b'base\n')
    git(root, 'add', 'file')
    git(root, 'commit', '-qm', 'base')

checks = []
for answer in ((b'n', b'y') if IS_C else (b'n', b'', b'\x1b', b'y')):
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        fixture(root)
        (root / 'file').write_bytes(b'private\x00edits\n')
        index = git(root, 'ls-files', '--stage', '-z')
        app = Terminal(root)
        app.send(b':5\r')
        app.send(b'!')
        assert b'revert' in app.output.lower(), app.output
        app.send(answer + (b'' if IS_C or answer == b'\x1b' else b'\r'))
        app.close()
        assert (root / 'file').read_bytes() == (b'base\n' if answer == b'y' else b'private\x00edits\n'), app.output
        assert git(root, 'ls-files', '--stage', '-z') == index
        if not IS_C:
            backups = list((root / '.git/tig-revert').glob('*/worktree'))
            assert len(backups) == (answer == b'y')
            if backups:
                assert backups[0].read_bytes() == b'private\x00edits\n'
        checks.append('ordinary-' + answer.decode())

if not IS_C:
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        fixture(root)
        (root / 'file').write_bytes(b'keep scripted edits\n')
        script = root / 'commands'
        script.write_text(':5\n:status-revert\ny\n:quit\n')
        result = subprocess.run([BINARY, '-C', str(root), 'status'],
            env=dict(os.environ, TIG_SCRIPT=str(script), TIGRC_SYSTEM=str(Path(__file__).resolve().parents[2] / 'tigrc'), TIGRC_USER='/dev/null'),
            capture_output=True, timeout=10)
        assert result.returncode == 0, result.stderr
        assert (root / 'file').read_bytes() == b'keep scripted edits\n'
        assert not (root / '.git/tig-revert').exists()
        checks.append('script-cannot-confirm')
    for side in ('ours', 'theirs', 'delete'):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            fixture(root)
            git(root, 'checkout', '-qb', 'side')
            (root / 'file').write_bytes(b'theirs\n')
            git(root, 'commit', '-qam', 'theirs')
            git(root, 'checkout', '-q', '-')
            if side == 'delete':
                git(root, 'rm', 'file')
            else:
                (root / 'file').write_bytes(b'ours\n')
            git(root, 'commit', '-qam', 'ours')
            git(root, 'merge', 'side', ok=False)
            index = git(root, 'ls-files', '--stage', '-z')
            original = (root / 'file').read_bytes()
            app = Terminal(root)
            app.send(b'/^U file\r')
            app.send(b'!')
            assert b'Choose :status-revert' in app.output, app.output
            assert (root / 'file').read_bytes() == original
            choice = 'ours' if side == 'delete' else side
            app.send((':status-revert ' + choice + '\r').encode())
            app.send(b'y\r')
            assert git(root, 'ls-files', '--stage', '-z') == index
            if side == 'delete':
                assert not (root / 'file').exists()
            else:
                assert (root / 'file').read_bytes() == (side + '\n').encode()
            app.send(b'u')
            app.close()
            assert not git(root, 'ls-files', '--unmerged')
            backups = list((root / '.git/tig-revert').glob('*/worktree'))
            assert len(backups) == 1 and backups[0].read_bytes() == original
            checks.append('conflict-' + side + '-then-stage')
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        fixture(root)
        git(root, 'checkout', '-qb', 'side')
        (root / 'file').write_bytes(b'theirs\n')
        git(root, 'commit', '-qam', 'theirs')
        git(root, 'checkout', '-q', '-')
        (root / 'file').write_bytes(b'ours\n')
        git(root, 'commit', '-qam', 'ours')
        git(root, 'merge', 'side', ok=False)
        git(root, 'config', 'merge.tool', 'tig-test')
        git(root, 'config', 'mergetool.tig-test.cmd', 'printf "resolved\\n" > "$MERGED"')
        git(root, 'config', 'mergetool.tig-test.trustExitCode', 'true')
        git(root, 'config', 'mergetool.prompt', 'false')
        original = (root / 'file').read_bytes()
        script = root / 'script'
        script.write_text('/^U file\n:status-merge\n:quit\n')
        scripted = subprocess.run([BINARY, '-C', str(root), 'status'],
            env=dict(os.environ, TIG_SCRIPT=str(script), TIGRC_SYSTEM=str(Path(__file__).resolve().parents[2] / 'tigrc'), TIGRC_USER='/dev/null'),
            capture_output=True, timeout=10)
        assert scripted.returncode != 0 and (root / 'file').read_bytes() == original
        assert git(root, 'ls-files', '--unmerged')
        app = Terminal(root)
        app.send(b'/^U file\rM')
        assert b'Run ' in app.output and b'mergetool' in app.output, app.output
        app.send(b'n\r')
        assert (root / 'file').read_bytes() == original
        app.send(b'My\r')
        deadline = time.monotonic() + 5
        while app.output.count(b'\x1b[?1049h') < 2 and time.monotonic() < deadline:
            app.drain()
        assert app.output.count(b'\x1b[?1049h') >= 2, app.output
        app.close()
        assert (root / 'file').read_bytes() == b'resolved\n'
        assert not git(root, 'ls-files', '--unmerged')
        checks.append('mergetool-confirmation-and-script-refusal')
print({'binary': BINARY, 'checks': checks, 'passed': len(checks)})
