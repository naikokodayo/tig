#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-2.0-or-later
"""Send real SGR mouse input; compare C/Rust exported viewport/selection state."""
import difflib
import fcntl
import os
from pathlib import Path
import pty
import re
import select
import struct
import subprocess
import sys
import tempfile
import termios
import time

ROOT = Path(__file__).resolve().parents[2]


def mouse(button, x, y):
    return f"\x1b[<{button};{x};{y}M".encode()


def run(binary, repo, home, config, keys):
    (home / 'tigrc').write_text('set main-view = commit-title:yes,graph=no,refs=no\n'
                              'set show-changes = no\n' + config)
    target = repo / 'screen'
    target.unlink(missing_ok=True)
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 18, 80, 0, 0))

    def setup():
        os.setsid()
        fcntl.ioctl(0, termios.TIOCSCTTY, 0)

    env = {k: v for k, v in os.environ.items() if not k.startswith(('TIG_', 'TIGRC_', 'GIT_'))}
    env.update(HOME=str(home), TERM='xterm-256color', TIGRC_SYSTEM='',
               TIGRC_USER=str(home / 'tigrc'), LC_ALL='en_US.UTF-8',
               GIT_CONFIG_GLOBAL='/dev/null', COLUMNS='80', LINES='18')
    process = subprocess.Popen([str(binary)], cwd=repo, env=env, stdin=slave,
                               stdout=slave, stderr=slave, preexec_fn=setup)
    os.close(slave)
    output = bytearray()

    def drain(seconds):
        deadline = time.monotonic() + seconds
        while time.monotonic() < deadline:
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
        while b'[main]' not in output and time.monotonic() < deadline:
            drain(0.05)
        assert b'[main]' in output, (binary, output)
        drain(1.0)  # C populates history asynchronously after drawing its first title.
        live_output = b""
        for chunk in [*keys, b':save-view screen\r', b'Q']:
            if chunk == b':save-view screen\r':
                live_output = bytes(output)
            os.write(master, chunk)
            drain(0.25)
            if chunk.startswith(b"\x1b[<") and int(chunk[3:].split(b";")[0]) < 64:
                os.write(master, chunk[:-1] + b"m")
                drain(0.1)
        assert process.wait(timeout=3) == 0 and target.exists(), (binary, output)
        screen = target.read_text()
        target.unlink()
        # C retains stale selected flags on offscreen rows; Position is authoritative.
        screen = re.sub(r" selected=[01]", "", screen)
        return screen, live_output
    finally:
        if process.poll() is None:
            process.kill()
            process.wait()
        os.close(master)


def main():
    binaries = [Path(arg).resolve() for arg in sys.argv[1:]] or [ROOT / 'src/tig', ROOT / 'target/release/tig']
    assert len(binaries) == 2, 'provide reference and candidate binaries'
    with tempfile.TemporaryDirectory(prefix='tig-mouse-') as temp:
        home = Path(temp)
        repo = home / 'repo'
        repo.mkdir()
        def git(*args):
            return subprocess.check_output(['git', '-C', str(repo), *args])
        git('init', '-q')
        git('config', 'user.name', 'Test')
        git('config', 'user.email', 'test@example.invalid')
        git('config', 'commit.gpgsign', 'false')
        for i in range(40):
            (repo / 'file').write_text(''.join(f'line {i} {j}\n' for j in range(45)))
            git('add', 'file')
            git('commit', '-qm', f'Commit {i:02}')
        cases = [
            ('default capture off', '', []),
            ('runtime capture toggle', '', [b':toggle mouse\r', mouse(0, 2, 4), b':toggle mouse\r']),
            ('click row', '', [mouse(0, 2, 4)]),
            ('same row opens', '', [mouse(0, 2, 4), mouse(0, 2, 4)]),
            ('title and status ignored', '', [mouse(0, 2, 17), mouse(65, 2, 18)]),
            ('outside width ignored', '', [mouse(0, 81, 4)]),
            ('scroll default', '', [mouse(65, 2, 4)]),
            ('scroll configured', 'set mouse-scroll = 5\n', [mouse(65, 2, 4)]),
            ('scroll up configured', 'set mouse-scroll = 5\n', [mouse(65, 2, 4), mouse(64, 2, 4)]),
            ('scroll clamp bottom', 'set mouse-scroll = 100\n', [mouse(65, 2, 4)]),
            ('scroll bottom boundary', 'set mouse-scroll = 100\n', [mouse(65, 2, 4), mouse(65, 2, 4)]),
            ('scroll top boundary', '', [mouse(64, 2, 4)]),
            ('cursor wheel', 'set mouse-wheel-cursor = yes\nset mouse-scroll = 5\n', [mouse(65, 2, 4)]),
            ('cursor wheel clamp', 'set mouse-wheel-cursor = yes\nset mouse-scroll = 100\n', [mouse(65, 2, 4)]),
            ('cursor wheel up', 'set mouse-wheel-cursor = yes\n', [mouse(0, 2, 8), mouse(64, 2, 4)]),
            ('cursor first boundary', 'set mouse-wheel-cursor = yes\n', [mouse(64, 2, 4)]),
            ('cursor last boundary', 'set mouse-wheel-cursor = yes\nset mouse-scroll = 100\n', [mouse(65, 2, 4), mouse(65, 2, 4)]),
            ('cursor crosses viewport', 'set mouse-wheel-cursor = yes\n', [mouse(0, 2, 15), mouse(65, 2, 4)]),
            ('scroll beyond partial viewport', 'set mouse-wheel-cursor = yes\nset mouse-scroll = 30\n', [mouse(65, 2, 4), b':toggle mouse-wheel-cursor\r', mouse(65, 2, 4)]),
            ('blank rows clamp without opening', '', [b':view-tree\r', mouse(0, 2, 16), mouse(0, 2, 16)]),
            ('middle button wheel', '', [mouse(1, 2, 4)]),
            ('horizontal focus only', 'set vertical-split = no\n', [b'\r', mouse(0, 2, 4)]),
            ('horizontal select after focus', 'set vertical-split = no\n', [b'\r', mouse(0, 2, 4), mouse(0, 2, 4)]),
            ('vertical focus only', 'set vertical-split = yes\n', [b'\r', mouse(0, 2, 4)]),
            ('vertical separator ignored', 'set vertical-split = yes\n', [b'\r', mouse(0, 41, 4)]),
            ('wheel focuses other pane only', 'set vertical-split = yes\n', [b'\r', mouse(65, 2, 4)]),
            ('diff repeat click inert', 'set vertical-split = yes\n', [b'\r', mouse(0, 45, 2), mouse(0, 45, 2)]),
            ('diff stat click jumps', 'set vertical-split = yes\n', [b'\r', mouse(0, 45, 9), mouse(0, 45, 9)]),
        ]
        failures = []
        for name, config, keys in cases:
            if name not in ('default capture off', 'runtime capture toggle'):
                config = 'set mouse = yes\n' + config
            results = [run(binary, repo, home, config, keys) for binary in binaries]
            for binary, (_, output) in zip(binaries, results):
                if name == 'default capture off':
                    assert not re.search(rb'\x1b\[\?[0-9;]*1000[0-9;]*h', output), (name, binary, 'capture enabled')
                else:
                    assert b'\x1b[?1003h' not in output, (name, binary, 'hover capture enabled')
                    assert re.search(rb'\x1b\[\?[0-9;]*1000[0-9;]*h', output), (name, binary, 'capture not enabled')
                if name == 'runtime capture toggle':
                    modes = re.findall(rb'\x1b\[\?[0-9;]*1000[0-9;]*([hl])', output)
                    assert modes == [b'h', b'l'], (binary, modes)
                if name in ('cursor first boundary', 'cursor last boundary'):
                    edge = 'first' if name == 'cursor first boundary' else 'last'
                    assert f'Cannot move beyond the {edge} line'.encode() in output, (name, binary)
            if results[0][0] != results[1][0]:
                failures.append(name)
                print('FAIL:', name)
                print(''.join(difflib.unified_diff(results[0][0].splitlines(True), results[1][0].splitlines(True), fromfile='C', tofile='Rust')))
            else:
                print('PASS:', name, flush=True)
        assert not failures, failures
        assert not git('status', '--porcelain'), 'mouse operations changed repository'
        print(f'PASS: {len(cases)} C/Rust real SGR mouse cases')


if __name__ == '__main__':
    main()
