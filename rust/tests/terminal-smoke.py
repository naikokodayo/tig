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

            trace = home / 'commands.trace'
            environment['TIG_TRACE'] = str(trace)

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
            newest = git('rev-parse', 'HEAD').strip()
            worktree_text = 'first line\nsecond line\nunstaged fixture\nsecond added fixture\n'
            (repo / 'fixture.txt').write_text(worktree_text)
            def start(args=(), cwd=None):
                nonlocal process, master, slave, release_read, release_write
                master, slave = pty.openpty()
                fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 24, 200, 0, 0))
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
                    observed = bytes(received)
                    if any(re.search(r'\[(main|diff|status|stage|tree|blob|blame)\]', text) for text in required):
                        # Ignore the preceding frame's tail; wait for the focused title redraw.
                        frame = observed.rfind(b'\x1b[1;1H\x1b[2J')
                        if frame < 0:
                            continue
                        observed = observed[frame:]
                        if not re.search(rb'\x1b\[7m\[(?:main|diff|status|stage|tree|blob|blame)\][^\x1b]*\x1b\[0m', observed):
                            continue
                    plain = ANSI.sub(b'', observed).decode(errors='replace')
                    if all(text in plain or (text == 'PTY_CHILD_PID=' and text.encode() in received) for text in required) and all(x in observed for x in raw_required):
                        evidence['checks'].append({'name': label, 'passed': True,
                                                   'observed': plain, 'bytes': len(received)})
                        return plain
                raise AssertionError(f'{label}: missing {required!r}; received {bytes(received)!r}')

            expect('working changes precede history', ['Unstaged changes', 'newest fixture commit',
                                                       'oldest fixture commit', '[main] Unstaged changes'])
            saved_options = home / 'saved options.tigrc'
            save_command = f':save-options "{saved_options}"\r'.encode()
            expect('save options to quoted path', ['Saved options to '], save_command)
            saved_bytes = saved_options.read_bytes()
            assert b'set log-options = --cc --stat' in saved_bytes
            hash_options = home / 'hash#options.tigrc'
            expect('save options keeps unquoted hash in path', ['Saved options to '],
                   f':save-options {hash_options}\r'.encode())
            assert not (home / 'hash').exists(), 'save-options wrote the truncated path'
            assert hash_options.read_bytes() == saved_bytes
            expect('save options refuses overwrite', ['Failed to save options:'], save_command)
            assert saved_options.read_bytes() == saved_bytes
            expect('save options reports missing directory', ['Failed to save options:'],
                   f':save-options "{home / "missing" / "options"}"\r'.encode())
            expect('hide working changes for history navigation',
                   ['newest fixture commit', 'oldest fixture commit', f'[main] {newest} - commit 1 of 2'],
                   b':set show-changes = no\r')
            expect('comma selects main parent', [f'[main] {oldest} - commit 2 of 2'], b',')
            expect('refresh preserves main history', [f'[main] {oldest} - commit 2 of 2'], b'R')
            expect('less-than restores main position', [f'[main] {newest} - commit 1 of 2'], b'<')
            expect('empty main history stays open', ['Already at start of history', '[main]'], b'<')
            expect('prompt command opens pager', ['[pager] echo navigation-output', 'navigation-output'],
                   b':!echo navigation-output\r')
            expect('command pager closes to main', [f'[main] {newest} - commit 1 of 2'], b'q')
            expect('j selects second commit', [f'[main] {oldest} - commit 2 of 2'], b'j')
            expect('Enter opens selected commit diff', ['[diff]', oldest, 'oldest fixture commit'], b'\r')
            expect('Tab focuses split parent', [f'[main] {oldest} - commit 2 of 2', '[diff]'], b'\t',
                   raw_required=(b'\x1b[7m[main]',))
            expect('main command refreshes displayed diff refs', ['Refs: <parent-refresh>', '[main]'],
                   b':exec @git tag parent-refresh %(commit)\r')
            expect('Tab focuses split child', [f'[main] {oldest} - commit 2 of 2', '[diff]'], b'\t',
                   raw_required=(b'\x1b[7m[diff]',))
            expect('K loads previous parent commit into diff', [f'[main] {newest} - commit 1 of 2', f'[diff] {newest}'], b'K')
            expect('J loads next parent commit into diff', [f'[main] {oldest} - commit 2 of 2', f'[diff] {oldest}'], b'J')
            expect('O maximizes focused diff', [f'[diff] {oldest}'], b'O',
                   raw_required=(b'\x1b[23;1H\x1b[7m[diff]',))
            expect('q returns to selected history row', [f'[main] {oldest} - commit 2 of 2'], b'q')
            expect('reopen split for command pager', ['[diff]', oldest], b'\r')
            expect('split command pager maximizes', ['[pager] echo split-output', 'split-output'],
                   b':!echo split-output\r', raw_required=(b'\x1b[23;1H\x1b[7m[pager]',))
            expect('split command pager closes to main', [f'[main] {oldest} - commit 2 of 2'], b'q')
            expect('search prompt', ['/'], b'/')
            expect('search selects matching commit', [f'[main] {newest} - commit 1 of 2'], b'newest\r')
            expect('status opens at header', ['Changes not staged for commit:', 'fixture.txt', '[status] Nothing to update'], b's')
            expect('select unstaged fixture', ["[status] Press u to stage 'fixture.txt' for commit"], b'jjjj')
            expect('u stages fixture', ["[status] Press u to unstage 'fixture.txt'"], b'u')
            assert git('diff', '--cached', '--name-only').strip() == 'fixture.txt'
            assert git('diff', '--name-only').strip() == ''
            evidence['checks'].append({'name': 'stage verified against Git index', 'passed': True})
            expect('Enter opens cached diff', ['+unstaged fixture', '[stage]'], b'\r')
            expect('R preserves cached diff', ['+unstaged fixture', '[stage]'], b'R')
            expect('back from cached diff', ["[status] Press u to unstage 'fixture.txt'"], b'q')
            expect('u unstages fixture', ["[status] Press u to stage 'fixture.txt' for commit"], b'u')
            assert git('diff', '--cached', '--name-only').strip() == ''
            assert git('diff', '--name-only').strip() == 'fixture.txt'
            evidence['checks'].append({'name': 'unstage verified against Git index', 'passed': True})
            expect('open two-line unstaged patch', ['+unstaged fixture', '+second added fixture', '[stage]'], b'\r')
            expect('line staging search prompt', ['/'], b'/')
            expect('select added line A', ['[stage]'], b'\\+unstaged fixture\r',
                   raw_required=(b'\x1b[7m+unstaged fixture',))
            expect('1 stages only selected line', ['+second added fixture', '[stage]'], b'1')
            cached = git('diff', '--cached', '--', 'fixture.txt')
            assert '+unstaged fixture' in cached and '+second added fixture' not in cached, cached
            assert git('show', ':fixture.txt') == 'first line\nsecond line\nunstaged fixture\n'
            assert (repo / 'fixture.txt').read_text() == worktree_text
            evidence['checks'].append({'name': 'single-line stage index and unchanged worktree',
                                       'passed': True, 'cached_diff': cached})
            expect('back to status after line stage', ["[status] Press u to stage 'fixture.txt' for commit"], b'q')
            expect('refresh partially staged status', ["[status] Press u to stage 'fixture.txt' for commit"], b'R')
            expect('select cached partial file', ["[status] Press u to unstage 'fixture.txt'"], b'kk')
            expect('open cached partial patch', ['+unstaged fixture', '[stage]'], b'\r')
            expect('line unstage search prompt', ['/'], b'/')
            expect('select cached added line A', ['[stage]'], b'\\+unstaged fixture\r',
                   raw_required=(b'\x1b[7m+unstaged fixture',))
            expect('1 unstages only selected line and closes empty stage', ['[status]'], b'1')
            assert git('diff', '--cached') == ''
            assert (repo / 'fixture.txt').read_text() == worktree_text
            evidence['checks'].append({'name': 'single-line unstage empty index and unchanged worktree', 'passed': True})
            expect('refresh fully unstaged status', ["[status] Press u to stage 'fixture.txt' for commit"], b'R')
            expect('open hunk for staging', ['+unstaged fixture', '+second added fixture', '[stage]'], b'\r')
            expect('hunk staging search prompt', ['/'], b'/')
            expect('select line within hunk', ['[stage]'], b'\\+unstaged fixture\r',
                   raw_required=(b'\x1b[7m+unstaged fixture',))
            expect('u stages complete hunk and closes empty stage', ['[status]'], b'u')
            cached = git('diff', '--cached')
            assert '+unstaged fixture' in cached and '+second added fixture' in cached, cached
            assert git('show', ':fixture.txt') == worktree_text
            assert (repo / 'fixture.txt').read_text() == worktree_text
            evidence['checks'].append({'name': 'whole-hunk stage includes both lines and preserves worktree',
                                       'passed': True, 'cached_diff': cached})
            expect('refresh fully staged status', ["[status] Press u to unstage 'fixture.txt'"], b'R')
            expect('open cached hunk', ['+unstaged fixture', '+second added fixture', '[stage]'], b'\r')
            expect('hunk unstaging search prompt', ['/'], b'/')
            expect('select cached hunk line', ['[stage]'], b'\\+unstaged fixture\r',
                   raw_required=(b'\x1b[7m+unstaged fixture',))
            expect('cached u unstages complete hunk and closes empty stage', ['[status]'], b'u')
            assert git('diff', '--cached') == ''
            assert (repo / 'fixture.txt').read_text() == worktree_text
            evidence['checks'].append({'name': 'whole-hunk unstage empties index and preserves worktree', 'passed': True})
            expect('back from status', [f'[main] {newest} - commit 1 of 2'], b'q')
            expect('external argv command prompt', [':'], b':')
            expect('silent external argv command returns to history', [f'[main] {newest} - commit 1 of 2'],
                   b'exec @git config --local pty.fixture "literal spaces ; $(touch should-not-exist)"\r')
            assert git('config', '--local', 'pty.fixture').strip() == 'literal spaces ; $(touch should-not-exist)'
            assert not (repo / 'should-not-exist').exists()
            evidence['checks'].append({'name': 'external command literal argv without implicit shell', 'passed': True})
            argv_path = 'argv space ; $(echo literal).txt'
            (repo / argv_path).write_text('literal argv fixture\n')
            expect('external path command prompt', [':'], b':')
            expect('external command handles literal special filename', [f'[main] {newest} - commit 1 of 2'],
                   f'exec @git add -- "{argv_path}"\r'.encode())
            assert git('diff', '--cached', '--name-only').strip() == argv_path
            assert git('show', ':' + argv_path) == 'literal argv fixture\n'
            evidence['checks'].append({'name': 'external argv stages exact spaced special-character filename', 'passed': True})
            expect('external cleanup command prompt', [':'], b':')
            expect('external argv cleanup returns to history', [f'[main] {newest} - commit 1 of 2'],
                   f'exec @git reset -- "{argv_path}"\r'.encode())
            assert git('diff', '--cached') == ''
            (repo / argv_path).unlink()
            expect('echo command prompt', [':'], b':')
            echoed = expect('plus command displays first stdout line',
                            [f'[main] {newest} - commit 1 of 2', 'echo first line'],
                            f'exec +"{sys.executable}" -c "print(\'echo first line\');print(\'hidden second line\')"\r'.encode(),
                            raw_required=(b'\x1b[24;1Hecho first line\x1b[0m',))
            assert 'hidden second line' not in echoed
            expect('foreground command prompt', [':'], b':')
            expect('foreground command waits for Enter', ['git version', 'Press Enter to continue'],
                   b'exec !git -c pty.foreground=foreground-private-marker --version\r')
            assert termios.tcgetattr(slave) == before
            expect('foreground command resumes raw UI after Enter', [f'[main] {newest} - commit 1 of 2'], b'\r')
            assert not termios.tcgetattr(slave)[3] & termios.ICANON
            expect('quick command prompt', [':'], b':')
            quick = expect('successful quick command resumes without Enter', [f'[main] {newest} - commit 1 of 2'],
                           b'exec >git -c pty.quick=quick-private-marker --version\r')
            assert 'Press Enter to continue' not in quick
            expect('failed quick command prompt', [':'], b':')
            expect('failed quick command still waits for Enter', ['Command exited with', 'Press Enter to continue'],
                   b'exec >git -c pty.failed=failed-private-marker not-a-real-pty-subcommand\r')
            assert termios.tcgetattr(slave) == before
            expect('failed command resumes after Enter', [f'[main] {newest} - commit 1 of 2'], b'\r')
            expect('move off HEAD before default H', [f'[main] {oldest} - commit 2 of 2'], b'j')
            expect('default H resolves HEAD and selects newest commit', [f'[main] {newest} - commit 1 of 2'], b'H',
                   raw_required=(f'\x1b[7m[main] {newest} - commit 1 of 2'.encode(),))
            expect('quoted key binding prompt', [':'], b':')
            expect('install quoted H command binding', [f'[main] {newest} - commit 1 of 2'],
                   b'bind main H +git -c "pty.binding=quoted H value" config --get pty.binding\r')
            expect('H preserves quoted argv and echoes result', [f'[main] {newest} - commit 1 of 2', 'quoted H value'], b'H',
                   raw_required=(b'\x1b[24;1Hquoted H value\x1b[0m',))
            evidence['checks'].append({'name': 'foreground canonical tty, resumed raw UI, echo and quoted argv verified', 'passed': True})
            traced = trace.read_bytes()
            assert b'pty.fixture literal spaces ; $(touch should-not-exist)' in traced
            assert b'pty.binding=quoted H value config --get pty.binding' in traced
            for marker in (b'foreground-private-marker', b'quick-private-marker', b'failed-private-marker'):
                assert marker not in traced, f'foreground argv leaked into TIG_TRACE: {marker!r}'
            evidence['checks'].append({'name': 'TIG_TRACE includes captured commands but excludes foreground argv', 'passed': True})
            tree_oid = git('rev-parse', 'HEAD:nested').strip()
            blob_oid = git('rev-parse', 'HEAD:fixture.txt').strip()
            expect('tree opens', ['fixture.txt', f'[tree] {tree_oid} - file 1 of 2'], b't')
            expect('select root blob', [f'[tree] {blob_oid} - file 2 of 2'], b'j')
            expect('Enter opens blob', ['first line', 'second line', '[blob] fixture.txt - line 1 of 2'], b'\r')
            expect('q returns to tree', [f'[tree] {blob_oid} - file 2 of 2'], b'q')
            expect('select nested tree', [f'[tree] {tree_oid} - file 1 of 2'], b'k')
            expect('open nested tree', ['Directory path /nested/', 'child.txt', '[tree] Open parent directory'], b'\r')
            expect('back restores parent tree', ['fixture.txt', f'[tree] {tree_oid} - file 1 of 2'], b'q')
            expect('R preserves parent tree', ['fixture.txt', 'nested', f'[tree] {tree_oid} - file 1 of 2'], b'R')
            fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 12, 60, 0, 0))
            os.killpg(process.pid, signal.SIGWINCH)
            expect('resize redraws clipped status at row 11', [f'[tree] {tree_oid} - file  100%'], raw_required=(b'\x1b[11;1H',))
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
            expect('+2 selects second row after working changes', [f'[main] {newest} - commit 1 of 2'])
            finish('+2 session quit')
            before, selector = start(('blame', '--', 'fixture.txt'))
            expect('blame -- file', ['first line', 'second line', f'[blame] {oldest}:fixture.txt - line 1 of 2'])
            finish('blame session quit')
            before, selector = start(cwd=repo / 'nested')
            expect('subdirectory main opens', ['[main]'])
            expect('first tree uses startup directory', ['Directory path /nested/', 'child.txt', '[tree] Open parent directory'], b't')
            expect('startup tree parent reaches worktree root', ['Directory path /', 'fixture.txt', f'[tree] {tree_oid} - file 1 of 2'], b'\r')
            finish('subdirectory tree session quit')
            environment.update(GIT_DIR=str(repo / '.git'), GIT_WORK_TREE=str(repo))
            assert subprocess.check_output(['git', 'rev-parse', '--show-prefix'], cwd=home,
                                           env=environment) == b'\n'
            before, selector = start(cwd=home)
            expect('external cwd with explicit Git environment opens main', ['[main]'])
            expect('external cwd opens root tree', ['Directory path /', 'fixture.txt', f'[tree] {tree_oid} - file 1 of 2'], b't')
            finish('external cwd tree session quit')
            environment.pop('GIT_DIR')
            environment.pop('GIT_WORK_TREE')
            before, selector = start(('blame', '--', 'child.txt'), repo / 'nested')
            expect('blame resolves subdirectory relative file', ['nested fixture line', f'[blame] {oldest}:nested/child.txt - line 1 of 1'])
            finish('subdirectory blame session quit')
            for termination in (signal.SIGTERM, signal.SIGHUP, signal.SIGINT):
                offset = len(transcript)
                before, selector = start()
                expect(termination.name + ' session history', ['[main] Unstaged changes', 'PTY_CHILD_PID='])
                child_pid = int(re.search(rb'PTY_CHILD_PID=(\d+)', transcript[offset:]).group(1))
                if termination == signal.SIGTERM:
                    expect('signal while search prompt active', ['/'], b'/')
                os.kill(child_pid, termination)
                finish(termination.name + ' exit', send=None, expected_exit=1)
            untracked = repo / 'new-untracked.txt'
            untracked.write_text('new file\n')
            before, selector = start()
            expect('untracked row precedes history', ['Untracked changes', 'Unstaged changes', '[main] Untracked changes'])
            expect('untracked row opens untracked-only status', ['new-untracked.txt', '[status] Nothing to update'], b'\r')
            expect('select untracked file', ["[status] Press u to stage 'new-untracked.txt' for addition"], b'jj')
            expect('stage last untracked file and return to main', ['Staged changes', 'Unstaged changes', '[main] Unstaged changes'], b'u')
            assert git('show', ':new-untracked.txt') == 'new file\n'
            assert untracked.read_text() == 'new file\n'
            evidence['checks'].append({'name': 'untracked staging changes only Git index', 'passed': True})
            finish('synthetic status session quit')
            git('reset', '-q', '--', 'new-untracked.txt')
            untracked.unlink()
            partial_text = 'FIRST\nsecond line\naddition one\naddition two\n'
            (repo / 'fixture.txt').write_text(partial_text)
            before, selector = start(('status',))
            expect('partial block status', ['[status] Nothing to update'])
            expect('select partial block file', ["[status] Press u to stage 'fixture.txt'"], b'jjjj')
            expect('open partial block patch', ['+FIRST', '+addition one', '+addition two', '[stage]'], b'\r')
            expect('split block search prompt', ['/'], b'/')
            expect('select block for splitting', ['[stage]'], b'\\+addition one\r',
                   raw_required=(b'\x1b[7m+addition one',))
            expect('backslash splits hunk with shared context', ['@@ -1,2 +1,2 @@', '@@ -2,1 +2,3 @@', '[stage]'], b'\\')
            assert git('diff', '--cached') == ''
            expect('split partial search prompt', ['/'], b'/')
            expect('select change within split hunk', ['[stage]'], b'\\+addition one\r',
                   raw_required=(b'\x1b[7m+addition one',))
            expect('2 stages contiguous block only', ['+FIRST', '[stage]'], b'2')
            assert git('show', ':fixture.txt') == 'first line\nsecond line\naddition one\naddition two\n'
            assert (repo / 'fixture.txt').read_text() == partial_text
            expect('partial block refreshes status without R', ["[status] Press u to stage 'fixture.txt'"], b'q')
            expect('select staged block', ["[status] Press u to unstage 'fixture.txt'"], b'kk')
            expect('open staged block', ['+addition one', '+addition two', '[stage]'], b'\r')
            expect('partial unstage search prompt', ['/'], b'/')
            expect('select block to unstage', ['[stage]'], b'\\+addition two\r',
                   raw_required=(b'\x1b[7m+addition two',))
            expect('2 unstages block and closes empty stage', ['[status]'], b'2')
            assert git('diff', '--cached') == ''
            assert (repo / 'fixture.txt').read_text() == partial_text
            evidence['checks'].append({'name': 'split and partial block stage/unstage preserve worktree and update only selected index block', 'passed': True})
            finish('partial block session quit')
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
