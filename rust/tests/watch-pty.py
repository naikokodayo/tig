#!/usr/bin/env python3
"""External commits must refresh an idle terminal only in periodic mode.
Run with an explicit C or Rust executable; stdlib, disposable repositories only.
"""
import errno
import fcntl
import os
from pathlib import Path
import pty
import select
import signal
import struct
import subprocess
import sys
import tempfile
import termios
import time

BINARY = Path(sys.argv[1]).resolve()
for mode in ('manual', 'after-command', 'periodic'):
    with tempfile.TemporaryDirectory(prefix='tig-watch-pty-') as temporary:
        root = Path(temporary)
        env = {k: v for k, v in os.environ.items()
               if not k.startswith(('GIT_', 'TIG_', 'TIGRC_', 'XDG_'))}
        config = root / 'tigrc'
        config.write_text(f'set refresh-mode = {mode}\nset refresh-interval = 1\n')
        env.update(HOME=str(root), TERM='xterm-256color', TIGRC_SYSTEM='',
                   TIGRC_USER=str(config), GIT_CONFIG_NOSYSTEM='1',
                   GIT_CONFIG_GLOBAL='/dev/null', GIT_AUTHOR_NAME='Watch',
                   GIT_COMMITTER_NAME='Watch', GIT_AUTHOR_EMAIL='watch@example.invalid',
                   GIT_COMMITTER_EMAIL='watch@example.invalid')

        def git(*args):
            subprocess.run(['git', *args], cwd=root, env=env, check=True,
                           stdout=subprocess.PIPE, stderr=subprocess.PIPE)

        git('init', '-q')
        git('commit', '--allow-empty', '-qm', 'watch-initial')
        fd, slave = pty.openpty()
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 24, 120, 0, 0))
        release_read, release_write = os.pipe()

        def controlling_tty():
            os.setsid()
            fcntl.ioctl(0, termios.TIOCSCTTY, 0)

        # Keep the controlling session alive until the parent releases it (macOS).
        launcher = ('import os, subprocess, sys; '
                    'code = subprocess.call([sys.argv[1]]); '
                    'print("WATCH_EXIT=" + str(code), flush=True); '
                    'os.read(int(sys.argv[2]), 1); sys.exit(code)')
        process = subprocess.Popen([sys.executable, '-c', launcher, str(BINARY), str(release_read)],
                                   cwd=root, env=env, pass_fds=(release_read,),
                                   stdin=slave, stdout=slave, stderr=slave,
                                   preexec_fn=controlling_tty)

        def read_until(needle, seconds):
            output = bytearray()
            deadline = time.monotonic() + seconds
            while time.monotonic() < deadline:
                if not select.select([fd], [], [], max(0, deadline - time.monotonic()))[0]:
                    break
                try:
                    chunk = os.read(fd, 65536)
                except OSError as error:
                    if error.errno == errno.EIO:
                        break
                    raise
                if not chunk:
                    break
                output.extend(chunk)
                if needle in output:
                    return True, bytes(output)
            return False, bytes(output)

        try:
            found, output = read_until(b'watch-initial', 5)
            assert found, (mode, 'initial display', output)
            # C watches second-resolution mtimes; cross that boundary first.
            read_until(b'never-present-marker', 1.2)
            git('commit', '--allow-empty', '-qm', 'watch-external')
            found, output = read_until(b'external', 3.5)
            assert found == (mode == 'periodic'), (mode, 'idle refresh', output)
            if mode != 'periodic':
                os.write(fd, b'R')
                found, output = read_until(b'external', 5)
                assert found, (mode, 'explicit refresh', output)
            git('commit', '--allow-empty', '-qm', 'watch-command-return')
            os.write(fd, b':exec @true\r')
            found, output = read_until(b'command-return', 3.5)
            assert found == (mode != 'manual'), (mode, 'command refresh', output)
            if mode == 'manual':
                os.write(fd, b'R')
                found, output = read_until(b'command-return', 5)
                assert found, (mode, 'manual refresh after command', output)
            print(f'PASS: {mode} idle, command, and explicit refresh policy')
        finally:
            try:
                try:
                    os.write(fd, b'Q')
                except OSError as error:
                    if error.errno != errno.EIO:
                        raise
                exited, _ = read_until(b'WATCH_EXIT=', 2)
                os.write(release_write, b'x')
                if not exited:
                    os.killpg(process.pid, signal.SIGKILL)
                process.wait(timeout=5)
            finally:
                if process.poll() is None:
                    os.killpg(process.pid, signal.SIGKILL)
                    process.wait(timeout=5)
                for descriptor in (fd, slave, release_read, release_write):
                    os.close(descriptor)
