#!/usr/bin/env python3
"""Focused C/Rust file finder PTY regression; no original assertions changed."""
import argparse
import fcntl
import os
from pathlib import Path
import pty
import select
import struct
import subprocess
import tempfile
import termios
import time


def session(binary, repo, env, steps, args=(), expected_code=0):
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 24, 90, 0, 0))
    def setup():
        os.setsid()
        fcntl.ioctl(0, termios.TIOCSCTTY, 0)
    proc = subprocess.Popen([str(binary), *args], cwd=repo, env=env,
                            stdin=slave, stdout=slave, stderr=slave, preexec_fn=setup)
    os.close(slave)
    transcript = bytearray()
    def until(expected):
        current = bytearray()
        deadline = time.monotonic() + 8
        while time.monotonic() < deadline:
            if select.select([master], [], [], .05)[0]:
                try:
                    chunk = os.read(master, 65536)
                except OSError:
                    break
                current.extend(chunk)
                transcript.extend(chunk)
                if expected in current:
                    return
            if proc.poll() is not None:
                break
        raise AssertionError(f'Missing {expected!r}: {bytes(transcript)!r}')
    try:
        if 'TIG_SCRIPT' not in env:
            until(b'[main]')
        for keys, expected in steps:
            if isinstance(keys, tuple):
                fcntl.ioctl(master, termios.TIOCSWINSZ, struct.pack('HHHH', *keys, 0, 0))
            else:
                os.write(master, keys)
            until(expected)
        if 'TIG_SCRIPT' not in env:
            os.write(master, b'Q')
        deadline = time.monotonic() + 5
        while proc.poll() is None and time.monotonic() < deadline:
            if select.select([master], [], [], .05)[0]:
                try:
                    transcript.extend(os.read(master, 65536))
                except OSError:
                    break
        assert proc.wait(timeout=1) == expected_code, bytes(transcript)
    finally:
        if proc.poll() is None:
            proc.kill()
            proc.wait()
        os.close(master)
    return bytes(transcript)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--c', type=Path, default=Path('src/tig'))
    parser.add_argument('--rust', type=Path, default=Path('target/release/tig'))
    parser.add_argument('--c-only', action='store_true')
    opts = parser.parse_args()
    with tempfile.TemporaryDirectory(prefix='tig-finder-pty-') as temp:
        root = Path(temp)
        repo = root / 'repo'
        repo.mkdir()
        home = root / 'home'
        home.mkdir()
        env = {k: v for k, v in os.environ.items()
               if not k.startswith(('GIT_', 'TIG_', 'TIGRC_', 'XDG_'))}
        env.update(HOME=str(home), TERM='xterm', TIGRC_SYSTEM='', TIGRC_USER='/dev/null',
                   GIT_CONFIG_NOSYSTEM='1', GIT_CONFIG_GLOBAL='/dev/null')
        def git(*args):
            return subprocess.check_output(['git', *args], cwd=repo, env=env)
        git('init', '-q')
        git('config', 'user.name', 'Finder Fixture')
        git('config', 'user.email', 'finder@example.invalid')
        (repo / 'nested').mkdir()
        for name, content in [('alpha.txt', 'ALPHA_CONTENT'), ('nested/beta.txt', 'BETA_CONTENT'),
                              ('nested/bravo.txt', 'BRAVO_CONTENT'), ('space tab\tline\n', 'CONTROL_CONTENT'),
                              ('é-file', 'UNICODE_CONTENT'), ('--option', 'OPTION_CONTENT')]:
            (repo / name).write_text(content + '\n')
        git('add', '.')
        git('commit', '-qm', 'finder fixture')
        git('commit', '--allow-empty', '-qm', 'second fixture')
        common = [
            [('nested/beta', b'BETA_CONTENT')],
            [('nb', b'BETA_CONTENT')],
            [('nb\x1bOB', b'BRAVO_CONTENT')],
            [('nbx\x7f', b'BETA_CONTENT')],
            [('é', b'UNICODE_CONTENT')],
            [('space', b'CONTROL_CONTENT')],
            [('--option', b'OPTION_CONTENT')],
        ]
        binaries = [('C', opts.c.resolve())]
        if not opts.c_only:
            binaries.append(('Rust', opts.rust.resolve()))
        checks = 0
        for label, binary in binaries:
            for case in common:
                query, marker = case[0]
                session(binary, repo, env, [(b':view-blob\r', b'Find file:'),
                                          ((query + '\r').encode(), marker)])
                checks += 1
            session(binary, repo, env, [(b':view-blob\r', b'Find file:'),
                                      (b'\x03', b'[main]')])
            checks += 1
            session(binary, repo, env, [(b':view-blob\r', b'Find file:'),
                                      (b'alpha\r', b'ALPHA_CONTENT'),
                                      (b':view-blob\r', b'Find file:'),
                                      (b'nb\r', b'nested/beta.txt - line'),
                                      (b'q', b'main]')])
            checks += 1
            print(f'{label}: {len(common) + 2} file finder PTY cases passed', flush=True)
        if not opts.c_only:
            binary = opts.rust.resolve()
            # Index-only raw filename works on macOS too, where the filesystem
            # itself refuses non-UTF-8 names. Git trees still preserve those bytes.
            oid = git('rev-parse', 'HEAD:alpha.txt').strip()
            git('update-index', '--add', '--cacheinfo', '100644', oid, b'raw-\xff')
            git('commit', '-qm', 'raw filename')
            session(binary, repo, env, [(b':view-blob\r', b'Find file:'),
                                      (b'raw\r', b'ALPHA_CONTENT')])
            checks += 1
            session(binary, repo, env, [(b':view-blob\r', b'Find file:'),
                                      (b'zzzz', b'file 0 of 0'),
                                      (b'\r', b'[main]')])
            checks += 1
            # Reopen from the blob itself; cancel restores the blob.
            session(binary, repo, env, [(b':view-blob\r', b'Find file:'),
                                      (b'alpha\r', b'ALPHA_CONTENT'),
                                      (b':view-blob\r', b'Find file:'),
                                      (b'\x03', b'ALPHA_CONTENT')])
            checks += 1
            display = root / 'script.screen'
            script = root / 'finder.script'
            script.write_text(f':view-blob\nnb<Down><Enter>\n:save-display {display}\n')
            session(binary, repo, {**env, 'TIG_SCRIPT': str(script)}, [])
            assert 'BRAVO_CONTENT' in display.read_text()
            checks += 1
            script.write_text(f':view-blob\nnb<Down><Enter>\n:view-blob\n<C-C>\n:save-display {display}\n')
            session(binary, repo, {**env, 'TIG_SCRIPT': str(script)}, [])
            assert 'BRAVO_CONTENT' in display.read_text()
            checks += 1
            script.write_text(f':enter\n:view-blob\nalpha<Enter>\n:view-close\n:view-close-no-quit\n:save-display {display}\n')
            session(binary, repo, {**env, 'TIG_SCRIPT': str(script)}, [])
            assert '[main]' in display.read_text() and '[diff]' not in display.read_text()
            checks += 1
            script.write_text(':view-blob\nunfinished\n')
            output = session(binary, repo, {**env, 'TIG_SCRIPT': str(script)}, [], expected_code=1)
            assert b'Unfinished scripted file finder input' in output
            checks += 1
            (repo / 'long').write_text('x' * 70 + 'WRAPPED_TAIL\n')
            git('add', 'long')
            git('commit', '-qm', 'long file')
            session(binary, repo, env, [(b':toggle wrap-lines\r', b'[main]'),
                                      (b':view-blob\r', b'Find file:'),
                                      (b'long\r', b'WRAPPED_TAIL'),
                                      (b':view-blob\r', b'Find file:'),
                                      ((24, 30), b'Find file:'),
                                      (b'\x03', b'WRAPPED_TAIL')])
            checks += 1
            print('Rust: raw filename, empty result, reopen/cancel, resize and script cases passed', flush=True)
        print(f'{checks} PTY checks passed')


if __name__ == '__main__':
    main()
