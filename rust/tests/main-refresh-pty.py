#!/usr/bin/env python3
"""Interactive main refresh: cancel/reap, retain, retry, failure, split, quit.
Run: python3 rust/tests/main-refresh-pty.py target/release/tig
Only Python stdlib; the Git wrapper never touches a real user repository.
"""
import errno
import fcntl
import os
from pathlib import Path
import pty
import select
import shutil
import signal
import struct
import subprocess
import sys
import tempfile
import termios
import time

binary = Path(sys.argv[1]).resolve()
real_git = shutil.which('git')
with tempfile.TemporaryDirectory(prefix='tig-refresh-') as temporary:
    root = Path(temporary)
    env = {k: v for k, v in os.environ.items()
           if not k.startswith(('GIT_', 'TIG_', 'TIGRC_', 'XDG_'))}
    config = root / 'tigrc'
    config.write_text('set refresh-mode = manual\nset show-changes = no\n')
    env.update(HOME=str(root), TERM='xterm-256color', TIGRC_SYSTEM='',
               TIGRC_USER=str(config), GIT_CONFIG_NOSYSTEM='1',
               GIT_CONFIG_GLOBAL='/dev/null', GIT_AUTHOR_NAME='Refresh',
               GIT_COMMITTER_NAME='Refresh', GIT_AUTHOR_EMAIL='refresh@example.invalid',
               GIT_COMMITTER_EMAIL='refresh@example.invalid')

    def git(*args):
        subprocess.run([real_git, *args], cwd=root, env=env, check=True,
                       stdout=subprocess.PIPE, stderr=subprocess.PIPE)

    git('init', '-q')
    git('commit', '--allow-empty', '-qm', 'refresh-old-screen')
    wrapper = root / 'bin'
    wrapper.mkdir()
    (wrapper / 'git').write_text(f'''#!{sys.executable}
import os, pathlib, subprocess, sys, time
root = pathlib.Path({str(root)!r})
if sys.argv[1:2] == ['log'] and (root / 'mode').exists():
    mode = (root / 'mode').read_text()
    # Both pipes exceed typical kernel buffers; no reader may wait on the child.
    data = subprocess.check_output([{real_git!r}, *sys.argv[1:]])
    os.write(2, b'e' * 262144)
    for _ in range(1024):
        sys.stdout.buffer.write(data)
    sys.stdout.buffer.flush()
    (root / 'pid').write_text(str(os.getpid()))
    if mode == 'fail':
        sys.exit(23)
    time.sleep(30)
os.execv({real_git!r}, [{real_git!r}, *sys.argv[1:]])
''')
    (wrapper / 'git').chmod(0o755)
    env['PATH'] = str(wrapper) + os.pathsep + env['PATH']
    fd, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 24, 120, 0, 0))
    release_read, release_write = os.pipe()

    def controlling_tty():
        os.setsid()
        fcntl.ioctl(0, termios.TIOCSCTTY, 0)

    launcher = ('import os, subprocess, sys; '
                'code = subprocess.call([sys.argv[1]]); '
                'print("REFRESH_EXIT=" + str(code), flush=True); '
                'os.read(int(sys.argv[2]), 1); sys.exit(code)')
    process = subprocess.Popen([sys.executable, '-c', launcher, str(binary), str(release_read)],
                               cwd=root, env=env, pass_fds=(release_read,),
                               stdin=slave, stdout=slave, stderr=slave,
                               preexec_fn=controlling_tty)

    buffered = bytearray()

    def until(needle, seconds=5):
        output = bytearray(buffered)
        buffered.clear()
        if needle in output:
            return bytes(output)
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
                return bytes(output)
        raise AssertionError((needle, bytes(output)[-2000:]))

    def delayed(mode='delay'):
        (root / 'pid').unlink(missing_ok=True)
        (root / 'mode').write_text(mode)
        os.write(fd, b'R')
        deadline = time.monotonic() + 5
        while not (root / 'pid').exists() and time.monotonic() < deadline:
            if select.select([fd], [], [], .01)[0]:
                buffered.extend(os.read(fd, 65536))
        assert (root / 'pid').exists(), 'Git pipes were not drained'
        return int((root / 'pid').read_text())

    def reaped(pid):
        try:
            os.kill(pid, 0)
        except ProcessLookupError:
            return
        raise AssertionError(f'Git child {pid} is still alive or unreaped')

    def await_reaped(pid):
        # Focus and resize remain responsive while background cleanup finishes.
        deadline = time.monotonic() + 2
        while time.monotonic() < deadline:
            try:
                reaped(pid)
                return
            except AssertionError:
                time.sleep(.01)
        reaped(pid)

    try:
        until(b'refresh-old-screen')
        git('commit', '--allow-empty', '-qm', 'refresh-new-screen')
        pid = delayed()
        os.write(fd, b'j,<H')
        time.sleep(.2)
        os.kill(pid, 0)  # In-view navigation must not silently cancel the job.
        started = time.monotonic()
        os.write(fd, b'z')
        output = until(b'Loading stopped', 2)
        elapsed = time.monotonic() - started
        reaped(pid)
        assert b'refresh-old-screen' in output and b'refresh-new-screen' not in output, output
        print(f'PASS: R/z responds in {elapsed:.3f}s, navigation stays live, drains both pipes, reaps child, retains old rows')
        (root / 'mode').unlink()
        os.write(fd, b'R')
        until(b'refresh-new-screen')
        print('PASS: next R successfully replaces the retained history')
        git('commit', '--allow-empty', '-qm', 'refresh-failed-screen')
        pid = delayed('fail')
        output = until(b'git exited with', 5)
        reaped(pid)
        assert b'refresh-new-screen' in output and b'refresh-failed-screen' not in output, output
        print('PASS: failed Git refresh retains the last successful rows')
        (root / 'mode').unlink()
        os.write(fd, b'\r')
        until(b'[diff]')
        os.write(fd, b'\t')
        until(b'[main]')
        pid = delayed()
        os.write(fd, b'j')
        os.kill(pid, 0)
        started = time.monotonic()
        os.write(fd, b'z')
        output = until(b'Loading stopped', 2)
        reaped(pid)
        assert b'[diff]' in output and b'refresh-new-screen' in output
        assert b'refresh-failed-screen' not in output
        print(f'PASS: split R/z responds in {time.monotonic() - started:.3f}s and retains both panes')
        git('commit', '--allow-empty', '-qm', 'split-new')
        (root / 'mode').unlink()
        os.write(fd, b'R')
        output = until(b'Loading history')
        if b'split-new' not in output:
            output += until(b'split-new')
        completed = output[output.rfind(b'split-new'):]
        if b'[diff]' not in completed:
            completed += until(b'[diff]')
        assert b'[diff]' in completed and b'[main]' in completed, output
        print('PASS: completed split refresh replaces only the main pane')
        pid = delayed('fail')
        output = until(b'git exited with', 5)
        reaped(pid)
        assert b'[diff]' in output[output.rfind(b'split-new'):] and b'split-new' in output, output
        print('PASS: failed split refresh retains both panes')
        pid = delayed()
        os.write(fd, b'\t')
        until(b'[diff]')
        await_reaped(pid)
        print('PASS: changing split focus cancels and reaps the stale refresh')
        os.write(fd, b'\t')
        until(b'[main]')
        pid = delayed()
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 24, 100, 0, 0))
        os.kill(process.pid, signal.SIGWINCH)
        time.sleep(.2)
        await_reaped(pid)
        print('PASS: resizing split panes cancels and reaps the stale refresh')
        (root / 'mode').unlink()
        os.write(fd, b'\tq')
        output = bytearray()
        deadline = time.monotonic() + 2
        quiet = time.monotonic() + .5
        while time.monotonic() < deadline and time.monotonic() < quiet:
            if select.select([fd], [], [], .05)[0]:
                output.extend(os.read(fd, 65536))
                quiet = time.monotonic() + .5
        closed = bytes(output).rsplit(b'\x1b[2J', 1)[-1]
        assert b'[main]' in closed and b'[diff]' not in closed, closed
        print('PASS: closing the child pane keeps the refreshed main view')
        os.write(fd, b'Q')
        until(b'REFRESH_EXIT=0', 2)
        print('PASS: quitting after split refresh restores the terminal')
    finally:
        os.write(release_write, b'x')
        try:
            process.wait(timeout=2)
        except subprocess.TimeoutExpired:
            os.killpg(process.pid, signal.SIGKILL)
            process.wait(timeout=5)
        for descriptor in (fd, slave, release_read, release_write):
            os.close(descriptor)
