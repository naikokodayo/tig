#!/usr/bin/env python3
"""Paired real keyboard/PTY regression for per-view unbinding and key sequences.

Build src/tig and target/release/tig first; emits JSON for the slice receipt.
"""
import errno
import fcntl
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import pty
import select
import signal
import struct
import subprocess
import tempfile
import termios
import time

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location('upstream', ROOT / 'rust/tests/upstream-suite.py')
harness = importlib.util.module_from_spec(spec)
spec.loader.exec_module(harness)


def run(binary):
    with tempfile.TemporaryDirectory(prefix='tig-key-bindings-') as temporary:
        repo = Path(temporary)
        env = harness.environment([])
        env.pop('TIG_SCRIPT', None)
        env.update(HOME=str(repo), GIT_CONFIG_GLOBAL='/dev/null', TIGRC_SYSTEM='',
                   TIGRC_USER=str(repo / 'tigrc'), TERM='xterm-256color')
        subprocess.run(['git', 'init', '-q', str(repo)], env=env, check=True)
        subprocess.run(['git', '-C', str(repo), '-c', 'user.name=Test',
                        '-c', 'user.email=test@example.invalid', '-c', 'commit.gpgsign=false',
                        'commit', '--allow-empty', '-qm', 'key binding fixture'], env=env, check=True)
        (repo / 'tigrc').write_text('set refresh-mode = manual\nbind main q none\n')
        executable = repo / 'z'
        executable.write_text('#!/bin/sh\ntouch command-prefix-executed\n')
        executable.chmod(0o755)
        env['PATH'] = str(repo) + os.pathsep + env['PATH']
        master, slave = pty.openpty()
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 24, 100, 0, 0))

        def controlling_tty():
            os.setsid()
            fcntl.ioctl(0, termios.TIOCSCTTY, 0)

        process = subprocess.Popen([str(binary)], cwd=repo, env=env, stdin=slave,
                                   stdout=slave, stderr=slave, preexec_fn=controlling_tty)
        transcript = bytearray()
        checks = []

        def wait(label, predicate):
            deadline = time.monotonic() + 5
            while not predicate():
                assert process.poll() is None or predicate(), (label, bytes(transcript))
                assert time.monotonic() < deadline, (label, bytes(transcript))
                if select.select([master], [], [], 0.05)[0]:
                    try:
                        transcript.extend(os.read(master, 65536))
                    except OSError as error:
                        if error.errno != errno.EIO:
                            raise
            checks.append(label)

        def drain():
            while select.select([master], [], [], 0.1)[0]:
                transcript.extend(os.read(master, 65536))

        def send(keys):
            os.write(master, keys)

        def marker(name, keys):
            send(keys)
            wait(name, lambda: (repo / name).exists())
            drain()

        def command(text):
            # Wait for the prompt before sending its contents: C may drain
            # queued typeahead when entering an incremental prompt.
            drain()
            offset = len(transcript)
            send(b':')
            wait('prompt: ' + text, lambda: (b'\x1b[?25h' if binary.parent.name == 'src'
                                                  else b'\x1b[2K:') in transcript[offset:])
            offset = len(transcript)
            send(text.encode() + b'\r')
            wait('completed: ' + text, lambda: b'\x1b[?25l' in transcript[offset:]
                 or b'\x1b[2J' in transcript[offset:])

        try:
            wait('main opened', lambda: b'key binding fixture' in transcript)
            send(b'q')
            command('exec @touch q-masked')
            wait('q masks default quit', lambda: (repo / 'q-masked').exists())
            command('bind generic qb :exec @touch generic-qb')
            marker('generic-qb', b'qb')
            command('bind main qa :exec @touch main-qa')
            marker('main-qa', b'qa')
            # An exact action takes precedence over a longer sequence.
            command('bind main qab :exec @touch wrong-longer-match')
            (repo / 'main-qa').unlink()
            marker('main-qa', b'qa')
            assert not (repo / 'wrong-longer-match').exists()
            offset = len(transcript)
            send(b'q')
            wait('sequence prefix waits', lambda: b'Keys:' in transcript[offset:])
            offset = len(transcript)
            send(b'\x1b')
            wait('escape redraw', lambda: b'\x1b[J' in transcript[offset:]
                 or b'\x1b[2J' in transcript[offset:])
            command('exec @touch escape-cancelled')
            wait('escape cancels prefix', lambda: (repo / 'escape-cancelled').exists())
            # A failed sequence consumes the mismatched key, then resets.
            command('bind main z :exec @touch wrong-fallback')
            send(b'qz')
            command('exec @touch mismatch-reset')
            wait('mismatch resets prefix', lambda: (repo / 'mismatch-reset').exists())
            assert not (repo / 'wrong-fallback').exists()
            # Unknown key text must never be interpreted as an exec request.
            for prefix in ('@', '!', '?', '<Lt>', '+', '>'):
                # Remove inherited single-key actions so each is a prefix.
                if prefix in ('?', '<Lt>'):
                    command(f'bind generic {prefix} none')
                command(f'bind main {prefix}a view-help')
                drain()
                offset = len(transcript)
                send(prefix.replace('<Lt>', '<').encode())
                wait(f'{prefix} waits for second key', lambda: b'Keys:' in transcript[offset:])
                offset = len(transcript)
                send(b'z')
                wait(f'{prefix}z rejected as unknown', lambda: b'Unknown key' in transcript[offset:]
                     or (repo / 'command-prefix-executed').exists())
                assert not (repo / 'command-prefix-executed').exists(), prefix
                assert not (repo / 'wrong-fallback').exists(), prefix
                checks.append(f'{prefix}z cannot launch executable on PATH')
            offset = len(transcript)
            command('view-help')
            wait('help opened', lambda: b'help]' in transcript[offset:])
            offset = len(transcript)
            send(b'q')
            wait('q still closes help', lambda: b'main]' in transcript[offset:])
            send(b'Q')
            wait('quit exits', lambda: process.poll() is not None)
            assert process.returncode == 0
            assert b'Not implemented in Rust yet: none' not in transcript
            return {'binary': str(binary.relative_to(ROOT)),
                    'binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(),
                    'checks': checks, 'exit_code': 0}
        finally:
            if process.poll() is None:
                os.killpg(process.pid, signal.SIGKILL)
            os.close(master)
            os.close(slave)
            process.wait(timeout=5)


if __name__ == '__main__':
    print(json.dumps([run(ROOT / path) for path in ('src/tig', 'target/release/tig')], indent=2))
