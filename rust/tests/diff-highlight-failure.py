#!/usr/bin/env python3
"""Pair filter failures through real terminals, retaining literal executable paths."""
import errno
import fcntl
import importlib.util
import json
import os
from pathlib import Path
import pty
import selectors
import signal
import struct
import subprocess
import sys
import tempfile
import termios
import time

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location('suite', ROOT / 'rust/tests/upstream-suite.py')
h = importlib.util.module_from_spec(spec)
spec.loader.exec_module(h)
checks = []
skips = []
with tempfile.TemporaryDirectory(prefix='tig-highlight-') as temporary:
    directory = Path(temporary)
    # Lossless repository path through both applications and the filter process.
    repo = directory / os.fsdecode(b'repo-\xff')
    try:
        repo.mkdir()
    except OSError as error:
        if error.errno != errno.EILSEQ:
            raise
        skips.append('filesystem rejects non-UTF-8 repository paths')
        repo = directory / 'repo-路径'
        repo.mkdir()
    env = h.environment([])
    env.update(GIT_CONFIG_GLOBAL='/dev/null', TIGRC_SYSTEM='', LC_ALL='C', TERM='xterm',
               COLUMNS='80', LINES='30', GIT_AUTHOR_DATE='2020-01-01T00:00:00+0000',
               GIT_COMMITTER_DATE='2020-01-01T00:00:00+0000')
    for args in [('init', '-q'), ('config', 'user.name', 'Test'),
                 ('config', 'user.email', 'test@example.invalid')]:
        subprocess.run(['git', '-C', str(repo), *args], env=env, check=True, capture_output=True)
    (repo / 'file').write_text('line\n')
    for args in [('add', 'file'), ('-c', 'commit.gpgsign=false', 'commit', '-qm', 'fixture')]:
        subprocess.run(['git', '-C', str(repo), *args], env=env, check=True, capture_output=True)

    cases = {
        'missing': None,
        'empty-failure': 'exit 7\n',
        'partial-failure': 'cat >/dev/null\nprintf "filtered output\\n"\nexit 7\n',
        'early-close': 'printf "filtered output\\n"\nexit 7\n',
        'stderr-success': 'cat >/dev/null\nprintf "filtered output\\n"\nprintf "hidden error\\n" >&2\n',
    }
    for name, body in cases.items():
        program = directory / f'{name} filter ; literal'
        if body is not None:
            program.write_text('#!/bin/sh\n' + body)
            program.chmod(0o755)
        (directory / 'tigrc').write_text(f'set diff-highlight = "{program}"\n')
        screens = {}
        for mode, binary in [('c', ROOT / 'src/tig'), ('rust', ROOT / 'target/release/tig')]:
            screen = directory / f'{name}-{mode}.screen'
            (directory / 'steps').write_text(f':save-display {screen}\n:quit\n')
            run_env = {**env, 'TIGRC_USER': str(directory / 'tigrc'),
                       'TIG_SCRIPT': str(directory / 'steps')}
            code, timed_out, transcript = h.terminal(
                [str(binary), '-C', str(repo), 'show', 'HEAD'], run_env, 10)
            assert code == 0 and not timed_out, (name, mode, code, transcript)
            assert 'hidden error' not in transcript and 'tig warning:' not in transcript, (name, mode, transcript)
            screens[mode] = screen.read_text()
        assert screens['c'] == screens['rust'], (name, screens)
        assert ('filtered output' in screens['c']) == (name not in ('missing', 'empty-failure'))
        checks.append(name)

    # Interactive quit after a missing filter restores the real terminal attributes.
    for mode, binary in [('c', ROOT / 'src/tig'), ('rust', ROOT / 'target/release/tig')]:
        (directory / 'tigrc').write_text('set diff-highlight = program-that-does-not-exist\n')
        run_env = {**env, 'TIGRC_USER': str(directory / 'tigrc')}
        master, slave = pty.openpty()
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 30, 80, 0, 0))
        before = termios.tcgetattr(slave)
        def controlling_tty():
            os.setsid()
            fcntl.ioctl(0, termios.TIOCSCTTY, 0)
        # Reuse terminal-smoke's session-holder pattern for macOS termios checks.
        release_read, release_write = os.pipe()
        launcher = ('import os, subprocess, sys; '
                    'child = subprocess.Popen([sys.argv[1], *sys.argv[3:]]); '
                    'code = child.wait(); '
                    'print("PTY_CHILD_EXIT=" + str(code), flush=True); '
                    'os.read(int(sys.argv[2]), 1); sys.exit(code)')
        process = subprocess.Popen([sys.executable, '-c', launcher, str(binary),
                                    str(release_read), '-C', str(repo), 'show', 'HEAD'],
                                   env=run_env, stdin=slave, stdout=slave, stderr=slave,
                                   pass_fds=(release_read,), preexec_fn=controlling_tty)
        try:
            output = bytearray()
            with selectors.DefaultSelector() as selector:
                selector.register(master, selectors.EVENT_READ)
                def expect(marker):
                    deadline = time.monotonic() + 10
                    while marker not in output and time.monotonic() < deadline:
                        if selector.select(0.05):
                            output.extend(os.read(master, 65536))
                    assert marker in output, (mode, output)
                expect(b'[diff]')
                os.write(master, b'Q')
                expect(b'PTY_CHILD_EXIT=0')
            assert termios.tcgetattr(slave) == before, mode
            os.write(release_write, b'x')
            assert process.wait(timeout=5) == 0, mode
            checks.append(f'{mode}-terminal-restored')
        finally:
            if process.poll() is None:
                os.killpg(process.pid, signal.SIGKILL)
                process.wait()
            for descriptor in (master, slave, release_read, release_write):
                os.close(descriptor)
print(json.dumps({'checks': checks, 'passed': len(checks), 'skips': skips}, sort_keys=True))
