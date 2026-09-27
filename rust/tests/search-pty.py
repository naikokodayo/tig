#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-2.0-or-later
"""C/Rust search status and colored cells through real controlling terminals.
Usage: python3 rust/tests/search-pty.py src/tig target/release/tig
The small xterm decoder handles only the sequences emitted by these fixtures.
"""
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


def screen(data):
    cells = {}
    x = y = 0
    fg = bg = None
    last = ' '
    for token in re.findall(r'\x1b\[[0-?]*[ -/]*[@-~]|\x1b[()][A-Z0-9]|\x1b.|[^\x1b]', data.decode()):
        if token.startswith('\x1b['):
            arg, op = token[2:-1], token[-1]
            if arg.startswith(('?', '>')):
                continue
            values = [int(v or 0) for v in arg.split(';')]
            n = values[0] or 1
            if op in 'Hf':
                y, x = n - 1, (values[1] or 1) - 1 if len(values) > 1 else 0
            elif op == 'G': x = n - 1
            elif op == 'd': y = n - 1
            elif op == 'A': y -= n
            elif op == 'B': y += n
            elif op == 'C': x += n
            elif op == 'D': x -= n
            elif op == 'J':
                cells = {p: c for p, c in cells.items() if values[0] != 2 and p < (y, x)}
            elif op == 'K':
                cells = {p: c for p, c in cells.items() if p[0] != y or
                         (values[0] == 0 and p[1] < x) or (values[0] == 1 and p[1] > x)}
            elif op == 'X':
                for col in range(x, x + n): cells.pop((y, col), None)
            elif op == 'm':
                i = 0
                while i < len(values):
                    v = values[i]
                    if v == 0: fg = bg = None
                    elif v == 39: fg = None
                    elif v == 49: bg = None
                    elif 30 <= v <= 37: fg = v - 30
                    elif 40 <= v <= 47: bg = v - 40
                    elif v in (38, 48) and values[i + 1] == 5:
                        if v == 38: fg = values[i + 2]
                        else: bg = values[i + 2]
                        i += 2
                    i += 1
            elif op == 'b':
                for _ in range(n):
                    cells[y, x] = (last, fg, bg)
                    x += 1
            else:
                assert op in 'hlrt', repr(token)
        elif token.startswith('\x1b'):
            assert token in ('\x1b(B', '\x1b(0', '\x1b=', '\x1b>'), repr(token)
        elif token == '\r': x = 0
        elif token == '\n': y += 1
        elif token == '\b': x -= 1
        elif token >= ' ':
            cells[y, x] = (token, fg, bg)
            last = token
            x += 1
    status = ''.join(cells.get((15, col), (' ',))[0] for col in range(100)).rstrip()
    highlighted = {(row, col): c[0] for (row, col), c in cells.items()
                   if row < 14 and c[1:] == (45, 123)}
    return status, highlighted


class Terminal:
    def __init__(self, binary, root):
        self.master, slave = pty.openpty()
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 16, 100, 0, 0))
        def setup():
            os.setsid()
            fcntl.ioctl(0, termios.TIOCSCTTY, 0)
        env = {**os.environ, 'TERM': 'xterm-256color', 'HOME': str(root),
               'TIGRC_SYSTEM': str(Path(__file__).resolve().parents[2] / 'tigrc'),
               'TIGRC_USER': str(root / 'config'), 'XDG_DATA_HOME': str(root),
               'GIT_CONFIG_GLOBAL': '/dev/null', 'GIT_CONFIG_NOSYSTEM': '1', 'LC_ALL': 'C'}
        env.pop('TIG_SCRIPT', None)
        env.pop('NO_COLOR', None)
        self.process = subprocess.Popen([str(binary)], cwd=root, env=env, stdin=slave,
                                       stdout=slave, stderr=slave, preexec_fn=setup)
        os.close(slave)
        self.data = b''
        deadline = time.monotonic() + 5
        while b'[main]' not in self.data and time.monotonic() < deadline:
            self.drain()
        assert b'[main]' in self.data, self.data

    def drain(self):
        end = time.monotonic() + .2
        while time.monotonic() < end:
            if select.select([self.master], [], [], .02)[0]:
                try:
                    self.data += os.read(self.master, 65536)
                except OSError:
                    break

    def send(self, keys):
        os.write(self.master, keys.encode())
        self.drain()
        return screen(self.data)

    def close(self):
        os.write(self.master, b'Q')
        self.drain()
        os.close(self.master)
        try:
            assert self.process.wait(timeout=3) == 0
        finally:
            if self.process.poll() is None:
                self.process.kill()
                self.process.wait(timeout=3)


def main():
    binaries = [Path(p).resolve() for p in sys.argv[1:]]
    assert len(binaries) == 2, __doc__
    with tempfile.TemporaryDirectory() as temp:
        root = Path(temp)
        def git(*args):
            subprocess.run(['git', '-C', str(root), *args], check=True, capture_output=True)
        git('init', '-q')
        (root / 'file').write_text('header\n' + 'x' * 110 + ' needle needle\nneedle\n')
        git('add', 'file')
        for message in reversed(['top', 'needle needle', 'NEEDLE', 'gap', 'needle']):
            git('-c', 'user.name=Test', '-c', 'user.email=test@example.invalid',
                '-c', 'commit.gpgsign=false', 'commit', '--allow-empty', '-qm', message)
        (root / 'config').write_text('set show-changes = no\nset main-view = commit-title:yes,graph=no,refs=no\n'
                                    'set ignore-case = no\ncolor search-result color45 color123\n')
        outputs = []
        for binary in binaries:
            app = Terminal(binary, root)
            snapshots = []
            try:
                cases = [('/needle\r', "Line 2 matches 'needle' (1 of 2)", 18),
                         ('n', "Line 5 matches 'needle' (2 of 2)", 18),
                         ('n', "Line 2 matches 'needle' (1 of 2)", 18),
                         ('N', "Line 5 matches 'needle' (2 of 2)", 18),
                         ('/\r', "Line 2 matches 'needle' (1 of 2)", 18),
                         ('/absent\r', "No match found for 'absent'", 0),
                         ('/[nN][eE]+[dD][lL][eE]\r', "Line 3 matches '[nN][eE]+[dD][lL][eE]' (2 of 3)", 24)]
                for keys, message, count in cases:
                    result = app.send(keys)
                    assert result[0] == message, (binary, keys, result)
                    assert len(result[1]) == count, (binary, keys, result)
                    snapshots.append(result)
                for setting, pattern, message, count in [
                    ('smart-case', 'needle', "Line 5 matches 'needle' (3 of 3)", 24),
                    ('smart-case', 'NEEDLE', "Line 3 matches 'NEEDLE' (1 of 1)", 6),
                    ('yes', 'NEEDLE', "Line 5 matches 'NEEDLE' (3 of 3)", 24)]:
                    app.send(f':set ignore-case = {setting}\r')
                    result = app.send(f'/{pattern}\r')
                    assert result[0] == message and len(result[1]) == count, (binary, result)
                    snapshots.append(result)
                result = app.send('R')
                assert not result[1], (binary, 'refresh must clear highlights', result)
                snapshots.append(result)
                app.send(':set wrap-search = no\r')
                result = app.send('n')
                assert result[0] == "No match found for 'NEEDLE'", (binary, result)
                snapshots.append(result)
                app.send(':set main-view = line-number:yes commit-title:yes,graph=no,refs=no\r')
                result = app.send('/1\r')
                assert result[0] == "No match found for '1'" and not result[1], (binary, result)
                snapshots.append(result)
                app.send(':0\r')
                result = app.send('/^needle$\r')
                assert result[0] == "Line 3 matches '^needle$' (1 of 2)" and not result[1], (binary, result)
                snapshots.append(result)
                app.send(':set wrap-lines = yes\r')
                app.send(':view-tree\r')
                app.send(':/file\r')
                app.send(':view-blob\r')
                result = app.send('/needle\r')
                assert result[0] == "Line 3 matches 'needle' (1 of 2)" and len(result[1]) == 18, (binary, result)
                snapshots.append(result)
                result = app.send('n')
                assert result[0] == "Line 4 matches 'needle' (2 of 2)" and len(result[1]) == 18, (binary, result)
                snapshots.append(result)
                result = app.send('/[\r')
                assert result[0].startswith('Search failed:') and not result[1], (binary, result)
            finally:
                app.close()
            outputs.append(snapshots)
        assert outputs[0] == outputs[1], outputs
        print(f'PASS: {len(outputs[0])} paired status/highlight snapshots')


if __name__ == '__main__':
    main()
