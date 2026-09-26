#!/usr/bin/env python3
"""Real controlling-PTY smoke checks; stdlib only, isolated disposable Git data.

Run after cargo build --release: python3 rust/tests/terminal-smoke.py
This checks implemented workflows, not equivalence with upstream Tig.
"""
import errno
import fcntl
import hashlib
import json
import os
from pathlib import Path
import pty
import re
import selectors
import signal
import struct
import subprocess
import sys
import tempfile
import termios
import time

ROOT = Path(__file__).resolve().parents[2]
BINARY = ROOT / 'target/release/tig'
EVIDENCE = ROOT / 'migration/evidence/terminal-smoke.json'
ANSI = re.compile(rb'\x1b\[[0-?]*[ -/]*[@-~]')


def main():
    evidence = {'scope': 'Implemented Rust UI workflows only; not upstream parity',
                'binary_sha256': hashlib.sha256(BINARY.read_bytes()).hexdigest(), 'checks': []}
    process = None
    master = slave = release_read = release_write = None
    transcript = bytearray()
    try:
        with tempfile.TemporaryDirectory(prefix='tig-terminal-') as temporary:
            repo = Path(temporary) / 'repo'
            repo.mkdir()
            home = Path(temporary) / 'home'
            home.mkdir()
            environment = {k: v for k, v in os.environ.items()
                           if not k.startswith(('GIT_', 'TIG_', 'TIGRC_', 'XDG_'))}
            environment.update(HOME=str(home), TERM='xterm-256color', PYTHONDONTWRITEBYTECODE='1',
                               TIGRC_SYSTEM='', TIGRC_USER='/dev/null',
                               GIT_CONFIG_NOSYSTEM='1', GIT_CONFIG_GLOBAL='/dev/null')

            def git(*args):
                return subprocess.check_output(['git', *args], cwd=repo, env=environment,
                                               stderr=subprocess.STDOUT).decode()

            git('init', '-q')
            git('config', 'user.name', 'PTY Fixture')
            git('config', 'user.email', 'pty@example.invalid')
            (repo / 'fixture.txt').write_text('first line\n')
            (repo / 'nested').mkdir()
            (repo / 'nested/child.txt').write_text('nested fixture line\n')
            git('add', 'fixture.txt', 'nested/child.txt')
            git('commit', '-qm', 'oldest fixture commit')
            oldest = git('rev-parse', 'HEAD').strip()
            (repo / 'fixture.txt').write_text('first line\nsecond line\n')
            git('commit', '-qam', 'newest fixture commit')
            worktree_text = 'first line\nsecond line\nunstaged fixture\nsecond added fixture\n'
            (repo / 'fixture.txt').write_text(worktree_text)
            def start(args=(), cwd=None):
                nonlocal process, master, slave, release_read, release_write
                master, slave = pty.openpty()
                fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 24, 100, 0, 0))
                before = termios.tcgetattr(slave)

                def controlling_tty():
                    os.setsid()
                    fcntl.ioctl(0, termios.TIOCSCTTY, 0)

                # Hold the session open so macOS permits termios measurement after exit.
                release_read, release_write = os.pipe()
                launcher = ('import os, subprocess, sys; '
                            'child = subprocess.Popen([sys.argv[1], *sys.argv[3:]]); '
                            'print("PTY_CHILD_PID=" + str(child.pid), flush=True); '
                            'code = child.wait(); '
                            'print("PTY_CHILD_EXIT=" + str(code), flush=True); '
                            'os.read(int(sys.argv[2]), 1); sys.exit(code)')
                process = subprocess.Popen([sys.executable, '-c', launcher, str(BINARY),
                                            str(release_read), *args], cwd=cwd or repo,
                                           env=environment, pass_fds=(release_read,),
                                           stdin=slave, stdout=slave, stderr=slave,
                                           preexec_fn=controlling_tty)
                selector = selectors.DefaultSelector()
                selector.register(master, selectors.EVENT_READ)
                return before, selector

            before, selector = start()

            def expect(label, required, send=None, raw_required=()):
                if send is not None:
                    os.write(master, send)
                received = bytearray()
                deadline = time.monotonic() + 5
                while time.monotonic() < deadline:
                    ready = selector.select(max(0, deadline - time.monotonic()))
                    if not ready:
                        break
                    try:
                        block = os.read(master, 65536)
                    except OSError as error:
                        if error.errno == errno.EIO:
                            break
                        raise
                    if not block:
                        break
                    received.extend(block)
                    transcript.extend(block)
                    plain = ANSI.sub(b'', bytes(received)).decode(errors='replace')
                    if all(text in plain for text in required) and all(x in received for x in raw_required):
                        evidence['checks'].append({'name': label, 'passed': True,
                                                   'observed': plain, 'bytes': len(received)})
                        return
                raise AssertionError(f'{label}: missing {required!r}; received {bytes(received)!r}')

            expect('history rendering', ['newest fixture commit', 'oldest fixture commit', '[main] 1 of 2'])
            expect('j selects second commit', ['[main] 2 of 2'], b'j')
            expect('Enter opens selected commit diff', ['[diff]', oldest, 'oldest fixture commit'], b'\r')
            expect('q returns to selected history row', ['[main] 2 of 2'], b'q')
            expect('search prompt', ['/'], b'/')
            expect('search selects matching commit', ['[main] 1 of 2'], b'newest\r')
            expect('status opens', ['Changes not staged for commit:', 'fixture.txt', '[status] 1 of 3'], b's')
            expect('status selects unstaged fixture', ['[status] 3 of 3'], b'jj')
            expect('u stages fixture', ['[status] 3 of 3'], b'u')
            assert git('diff', '--cached', '--name-only').strip() == 'fixture.txt'
            assert git('diff', '--name-only').strip() == ''
            evidence['checks'].append({'name': 'stage verified against Git index', 'passed': True})
            expect('select staged fixture', ['[status] 2 of 3'], b'k')
            expect('Enter opens cached diff', ['+unstaged fixture', '[stage]'], b'\r')
            expect('R preserves cached diff', ['+unstaged fixture', '[stage]'], b'R')
            expect('back from cached diff', ['[status] 2 of 3'], b'q')
            expect('u unstages fixture', ['[status] 2 of 3'], b'u')
            assert git('diff', '--cached', '--name-only').strip() == ''
            assert git('diff', '--name-only').strip() == 'fixture.txt'
            evidence['checks'].append({'name': 'unstage verified against Git index', 'passed': True})
            expect('select unstaged file for line staging', ['[status] 3 of 3'], b'j')
            expect('open two-line unstaged patch', ['+unstaged fixture', '+second added fixture', '[stage]'], b'\r')
            expect('line staging search prompt', ['/'], b'/')
            expect('select added line A', ['[stage]'], b'+unstaged fixture\r',
                   raw_required=(b'\x1b[7m+unstaged fixture',))
            expect('1 stages only selected line', ['+second added fixture', '[stage]'], b'1')
            cached = git('diff', '--cached', '--', 'fixture.txt')
            assert '+unstaged fixture' in cached and '+second added fixture' not in cached, cached
            assert git('show', ':fixture.txt') == 'first line\nsecond line\nunstaged fixture\n'
            assert (repo / 'fixture.txt').read_text() == worktree_text
            evidence['checks'].append({'name': 'single-line stage index and unchanged worktree',
                                       'passed': True, 'cached_diff': cached})
            expect('back to status after line stage', ['[status] 3 of 3'], b'q')
            expect('refresh partially staged status', ['[status] 3 of 4'], b'R')
            expect('select cached partial file', ['[status] 2 of 4'], b'k')
            expect('open cached partial patch', ['+unstaged fixture', '[stage]'], b'\r')
            expect('line unstage search prompt', ['/'], b'/')
            expect('select cached added line A', ['[stage]'], b'+unstaged fixture\r',
                   raw_required=(b'\x1b[7m+unstaged fixture',))
            expect('1 unstages only selected line', ['[stage] 0 of 0'], b'1')
            assert git('diff', '--cached') == ''
            assert (repo / 'fixture.txt').read_text() == worktree_text
            evidence['checks'].append({'name': 'single-line unstage empty index and unchanged worktree', 'passed': True})
            expect('back after line unstage', ['[status] 2 of 4'], b'q')
            expect('refresh fully unstaged status', ['[status] 2 of 3'], b'R')
            expect('select file for hunk staging', ['[status] 3 of 3'], b'j')
            expect('open hunk for staging', ['+unstaged fixture', '+second added fixture', '[stage]'], b'\r')
            expect('hunk staging search prompt', ['/'], b'/')
            expect('select line within hunk', ['[stage]'], b'+unstaged fixture\r',
                   raw_required=(b'\x1b[7m+unstaged fixture',))
            expect('u stages complete hunk', ['[stage] 0 of 0'], b'u')
            cached = git('diff', '--cached')
            assert '+unstaged fixture' in cached and '+second added fixture' in cached, cached
            assert git('show', ':fixture.txt') == worktree_text
            assert (repo / 'fixture.txt').read_text() == worktree_text
            evidence['checks'].append({'name': 'whole-hunk stage includes both lines and preserves worktree',
                                       'passed': True, 'cached_diff': cached})
            expect('back after hunk stage', ['[status] 3 of 3'], b'q')
            expect('refresh fully staged status', ['[status] 3 of 3'], b'R')
            expect('select staged hunk file', ['[status] 2 of 3'], b'k')
            expect('open cached hunk', ['+unstaged fixture', '+second added fixture', '[stage]'], b'\r')
            expect('hunk unstaging search prompt', ['/'], b'/')
            expect('select cached hunk line', ['[stage]'], b'+unstaged fixture\r',
                   raw_required=(b'\x1b[7m+unstaged fixture',))
            expect('cached u unstages complete hunk', ['[stage] 0 of 0'], b'u')
            assert git('diff', '--cached') == ''
            assert (repo / 'fixture.txt').read_text() == worktree_text
            evidence['checks'].append({'name': 'whole-hunk unstage empties index and preserves worktree', 'passed': True})
            expect('back after hunk unstage', ['[status] 2 of 3'], b'q')
            expect('back from status', ['[main] 1 of 2'], b'q')
            expect('tree opens', ['fixture.txt', '[tree] 1 of 2'], b't')
            expect('select root blob', ['[tree] 2 of 2'], b'j')
            expect('Enter opens blob', ['first line', 'second line', '[blob] 1 of 2'], b'\r')
            expect('q returns to tree', ['[tree] 2 of 2'], b'q')
            expect('select nested tree', ['[tree] 1 of 2'], b'k')
            expect('open nested tree', ['nested/child.txt', '[tree] 1 of 1'], b'\r')
            expect('back restores parent tree', ['fixture.txt', '[tree] 1 of 2'], b'q')
            expect('R preserves parent tree', ['fixture.txt', 'nested', '[tree] 1 of 2'], b'R')
            fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 12, 60, 0, 0))
            os.killpg(process.pid, signal.SIGWINCH)
            expect('resize redraws status at row 11', ['[tree] 1 of 2'], raw_required=(b'\x1b[11;1H',))
            def finish(label, send=b'Q', expected_exit=0):
                nonlocal process, master, slave, release_read, release_write
                expect(label, [f'PTY_CHILD_EXIT={expected_exit}'], send,
                       raw_required=(b'\x1b[?1049l', b'\x1b[?25h'))
                after = termios.tcgetattr(slave)
                os.write(release_write, b'x')
                exit_code = process.wait(timeout=5)
                evidence['checks'].append({'name': label + ' terminal restoration',
                                           'passed': exit_code == expected_exit and before == after,
                                           'exit_code': exit_code,
                                           'terminal_flags_before': before[:6],
                                           'terminal_flags_after': after[:6],
                                           'all_attributes_restored': before == after})
                assert exit_code == expected_exit
                assert before == after, 'termios attributes were not restored'
                selector.close()
                for descriptor in (master, slave, release_read, release_write):
                    os.close(descriptor)
                process = master = slave = release_read = release_write = None

            finish('quit leaves alternate screen and restores cursor')
            before, selector = start(('+2',))
            expect('+2 sets initial selection', ['[main] 2 of 2'])
            finish('+2 session quit')
            before, selector = start(('blame', '--', 'fixture.txt'))
            expect('blame -- file', ['first line', 'second line', '[blame] 1 of 2'])
            finish('blame session quit')
            before, selector = start(('blame', '--', 'child.txt'), repo / 'nested')
            expect('blame resolves subdirectory relative file', ['nested fixture line', '[blame] 1 of 1'])
            finish('subdirectory blame session quit')
            for termination in (signal.SIGTERM, signal.SIGHUP, signal.SIGINT):
                offset = len(transcript)
                before, selector = start()
                expect(termination.name + ' session history', ['[main] 1 of 2', 'PTY_CHILD_PID='])
                child_pid = int(re.search(rb'PTY_CHILD_PID=(\d+)', transcript[offset:]).group(1))
                if termination == signal.SIGTERM:
                    expect('signal while search prompt active', ['/'], b'/')
                os.kill(child_pid, termination)
                finish(termination.name + ' exit', send=None, expected_exit=1)
            evidence['passed'] = True

    except Exception as error:
        evidence.update(passed=False, error=str(error))
        raise
    finally:
        if process is not None and process.poll() is None:
            process.kill()
            process.wait(timeout=5)
        for descriptor in (master, slave, release_read, release_write):
            if descriptor is not None:
                os.close(descriptor)
        evidence['transcript_sha256'] = hashlib.sha256(transcript).hexdigest()
        EVIDENCE.parent.mkdir(parents=True, exist_ok=True)
        EVIDENCE.write_text(json.dumps(evidence, indent=2) + '\n')
    print(f"{len(evidence['checks'])} PTY checks passed; evidence: {EVIDENCE}")


if __name__ == '__main__':
    main()
