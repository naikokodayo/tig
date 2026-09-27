#!/usr/bin/env python3
"""Focused C/Rust save-view comparison through real PTYs; no original assertions changed."""
import argparse
import hashlib
import importlib.util
import json
import os
import fcntl
import pty
import select
import struct
import termios
import time
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location('suite', ROOT / 'rust/tests/upstream-suite.py')
h = importlib.util.module_from_spec(spec)
spec.loader.exec_module(h)
p = argparse.ArgumentParser(description=__doc__)
p.add_argument('--before', action='store_true')
p.add_argument('--output', type=Path, required=True)
a = p.parse_args()
a.output.unlink(missing_ok=True)
results = {}
known_differences = {}
with tempfile.TemporaryDirectory(prefix='tig-general-export-') as temporary:
    directory = Path(temporary)
    repo = directory / 'repo'
    repo.mkdir()
    env = h.environment([])
    config = directory / 'tigrc'
    script = directory / 'steps'
    env.update(TIGRC_SYSTEM=str(ROOT / 'tigrc'), TIGRC_USER=str(config), TIG_SCRIPT=str(script),
               COLUMNS='80', LINES='30', LC_ALL='C',
               GIT_AUTHOR_DATE='2020-01-01T00:00:00+0000', GIT_COMMITTER_DATE='2020-01-01T00:00:00+0000')
    def git(*args):
        return subprocess.check_output(['git', '-C', str(repo), *args], env=env).decode().strip()
    git('init', '-q')
    git('config', 'user.name', 'Export Tester')
    git('config', 'user.email', 'export@example.invalid')
    git('config', 'commit.gpgsign', 'false')
    (repo / 'dir').mkdir()
    (repo / 'dir/nested').write_text('nested\n')
    (repo / 'file').write_text('content\nold\n')
    git('add', '.')
    git('commit', '-qm', 'base')
    (repo / 'file').write_text('content\nnew\n')
    git('commit', '-qam', 'second')
    (repo / 'file').write_text('content\nstashed\n')
    git('stash', 'push', '-qm', 'example')
    (repo / 'file').write_text('content\nmodified\n')
    (repo / 'untracked').write_text('extra\n')
    cases = [
        ('main', [], '', ''),
        ('main-changes', [], '', 'set show-changes = yes\n'),
        ('status', ['status'], '', ''),
        ('status-file', ['status'], ':move-down\n' * 4, ''),
        ('refs', ['refs'], '', ''),
        ('refs-selected', ['refs'], ':move-down\n', ''),
        ('tree', [], ':view-tree\n', ''),
        ('tree-header', [], ':view-tree\n:move-first-line\n', ''),
        ('blob', [], ':view-tree\n:move-last-line\n:enter\n', ''),
        ('blame', ['blame', 'file'], '', ''),
        ('grep', ['grep', 'content'], '', ''),
        ('grep-selected', ['grep', 'content'], ':move-down\n', ''),
        ('help', [], ':view-help\n', ''),
        ('help-collapsed', [], ':view-help\n:move-down\n:enter\n', ''),
        ('help-expanded', [], ':view-help\n:move-down\n:enter\n:enter\n', ''),
        ('reflog', ['reflog'], '', ''),
        ('stash', ['stash'], '', ''),
        ('log', ['log'], '', ''),
        ('log-patch', ['log', '-p', '--stat'], '', ''),
        ('log-graph', ['log', '--graph', '--stat'], '', ''),
        ('stage', ['status'], ':move-down\n' * 3 + ':enter\n', ''),
        ('diff', ['show', 'HEAD'], '', ''),
        ('pager', [], '', ''),
    ]
    for name, args, steps, extra_config in cases:
        config.write_text('set show-changes = no\n' + extra_config)
        captures = {}
        for mode in ('c', 'rust'):
            binary = ROOT / ('src/tig' if mode == 'c' else 'target/release/tig')
            out = directory / f'{name}-{mode}.data'
            script.write_text(steps + f':save-view {out}\n:quit\n')
            command = [str(binary), '-C', str(repo), *args]
            if name == 'pager':
                source = directory / 'input'
                source.write_text('plain content\n+added\n-deleted\n\n')
                command = ['/bin/sh', '-c', 'exec "$1" -C "$2" < "$3"', 'sh', str(binary), str(repo), str(source)]
            code, timed_out, transcript = h.terminal(command, env, 15)
            captures[mode] = dict(exit=code, timeout=timed_out, data=out.read_text() if out.exists() else None)
            if code or timed_out:
                captures[mode]['transcript'] = transcript
        c, rust = captures['c'], captures['rust']
        passed = not c['exit'] and not c['timeout'] and c['data'] is not None
        if a.before and name != 'diff':
            passed &= rust['exit'] != 0 and rust['data'] is None and not rust['timeout']
        else:
            passed &= not rust['exit'] and not rust['timeout'] and rust['data'] == c['data']
        results[name] = dict(passed=bool(passed), captures=captures)
        print(name, 'PASS' if passed else 'FAIL', flush=True)
    if not a.before:
        for name, extra, source in [
            ('word-diff', 'set word-diff = yes\n', None),
            ('ansi', '', 'plain\x1b[31mred\n'),
        ]:
            config.write_text(extra)
            target = directory / ('unsupported-' + name)
            target.write_text('untouched\n')
            script.write_text(f':save-view {target}\n:quit\n')
            command = [str(ROOT / 'target/release/tig'), '-C', str(repo)]
            if name == 'word-diff':
                command += ['show', 'HEAD']
            if source is not None:
                input_file = directory / 'ansi-input'
                input_file.write_text(source)
                command = ['/bin/sh', '-c', 'exec "$1" -C "$2" < "$3"', 'sh', str(ROOT / 'target/release/tig'), str(repo), str(input_file)]
            code, timed_out, transcript = h.terminal(command, env, 15)
            passed = code != 0 and not timed_out and 'save-view does not support' in transcript and target.read_text() == 'untouched\n'
            results['unsupported-' + name] = dict(passed=passed, exit=code, timeout=timed_out, transcript=transcript)
    # Destination checks exercise the same user action, not a separate writer.
    config.write_text('')
    for kind in ('existing', 'symlink', 'dangling', 'directory', 'missing-parent', 'quoted-path'):
        target = directory / f'safety {kind}'
        victim = directory / f'victim-{kind}'
        victim.write_text('untouched\n')
        if kind == 'existing':
            target.write_text('untouched\n')
        elif kind == 'symlink':
            target.symlink_to(victim)
        elif kind == 'dangling':
            victim.unlink()
            target.symlink_to(victim)
        elif kind == 'directory':
            target.mkdir()
        elif kind == 'missing-parent':
            target = target / 'child'
        script.write_text(f':save-view "{target}"\n:quit\n')
        code, timed_out, transcript = h.terminal([str(ROOT / 'target/release/tig'), '-C', str(repo), 'show', 'HEAD'], env, 15)
        if a.before:
            continue
        if kind == 'quoted-path':
            passed = code == 0 and target.read_text().startswith('View: diff\n')
        else:
            passed = 'Failed to save view' in transcript and (not victim.exists() if kind == 'dangling' else victim.read_text() == 'untouched\n')
            if kind == 'existing':
                passed &= target.read_text() == 'untouched\n'
            if kind == 'missing-parent':
                passed &= not target.exists()
        results['path-' + kind] = dict(passed=bool(passed and not timed_out), exit=code, timeout=timed_out)
        if not passed:
            results['path-' + kind]['transcript'] = transcript

    if not a.before:
        master, slave = pty.openpty()
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 30, 80, 0, 0))
        def tty():
            os.setsid()
            fcntl.ioctl(0, termios.TIOCSCTTY, 0)
        interactive_env = dict(env)
        interactive_env.pop('TIG_SCRIPT')
        process = subprocess.Popen([str(ROOT / 'target/release/tig'), '-C', str(repo)],
                                   env=interactive_env, stdin=slave, stdout=slave, stderr=slave, preexec_fn=tty)
        os.close(slave)
        transcript = bytearray()
        def wait_for(marker):
            deadline = time.monotonic() + 5
            while marker not in transcript and time.monotonic() < deadline:
                if select.select([master], [], [], .05)[0]:
                    transcript.extend(os.read(master, 65536))
            assert marker in transcript, transcript.decode(errors='replace')
        def send(keys):
            transcript.clear()
            os.write(master, keys)
            time.sleep(.1)
        try:
            wait_for(b'[main]')
            destination = repo / 'interactive data'
            send((':save-view "' + destination.name + '"').encode())
            wait_for(b'interactive data"')
            send(b'\x1b')
            wait_for(b'[main]')
            assert not destination.exists()
            send((':save-view "' + destination.name + '"\r').encode())
            wait_for(b'Saved view to')
            original = destination.read_bytes()
            assert original.startswith(b'View: main\n')
            send((':save-view "' + destination.name + '"\r').encode())
            wait_for(b'Failed to save view')
            assert destination.read_bytes() == original
            send(b'Q')
            deadline = time.monotonic() + 5
            while process.poll() is None and time.monotonic() < deadline:
                if select.select([master], [], [], .05)[0]:
                    try:
                        transcript.extend(os.read(master, 65536))
                    except OSError:
                        break
            assert process.wait(timeout=1) == 0
            results['interactive-cancel-create-refuse-overwrite'] = dict(passed=True)
        finally:
            os.close(master)
            if process.poll() is None:
                process.kill()
            process.wait(timeout=5)

    if not a.before:
        # Main annotations come from the same log stream, including selected refs.
        git('notes', 'add', '-m', 'review note\n\ncommit fake\nmore note text')
        git('notes', '--ref=review', 'add', '-m', 'custom note', 'HEAD^')
        for name, setting, steps, annotated in [
            ('default', '', '', [0]),
            ('disabled', 'no', '', []),
            ('custom', 'refs/notes/review', '', [0, 1]),
            ('missing', 'refs/notes/missing', '', [0]),
            ('toggle-off', 'yes', ':toggle show-notes\n', []),
            ('toggle-on', 'no', ':toggle show-notes\n', [0]),
            ('refresh', 'yes', ':refresh\n', [0]),
        ]:
            config.write_text('set show-changes = no\n' +
                              (f'set show-notes = {setting}\n' if setting else ''))
            captures = {}
            screens = {}
            for mode, binary in [('c', ROOT / 'src/tig'), ('rust', ROOT / 'target/release/tig')]:
                target = directory / f'notes-{name}-{mode}'
                screen = directory / f'notes-{name}-{mode}.screen'
                script.write_text(steps + f':save-display {screen}\n:save-view {target}\n:quit\n')
                code, timed_out, transcript = h.terminal([str(binary), '-C', str(repo)], env, 15)
                assert code == 0 and not timed_out, transcript
                captures[mode] = target.read_text()
                screens[mode] = screen.read_text()
            expected = ['main-commit', 'main-commit']
            for index in annotated:
                expected[index] = 'main-annotated'
            for index, kind in enumerate(expected):
                assert f'line[{index:3}] type={kind} selected={int(index == 0)}' in captures['c'], captures
            results['notes-' + name] = dict(passed=captures['rust'] == captures['c'] and
                                           screens['rust'] == screens['c'], captures=captures, screens=screens)

        git('update-index', '--add', '--cacheinfo', '160000,' + git('rev-parse', 'HEAD') + ',nested')
        git('commit', '-qm', 'gitlink')
        for mode, binary in [('c', ROOT / 'src/tig'), ('rust', ROOT / 'target/release/tig')]:
            target = directory / ('gitlink-' + mode)
            script.write_text(f':view-tree\n:save-view {target}\n:quit\n')
            code, timed_out, transcript = h.terminal([str(binary), '-C', str(repo)], env, 15)
            assert code == 0 and not timed_out, transcript
            data = target.read_text()
            assert data.startswith('View: tree\n') and data.count('type=default selected=') == 1, data
        # Entry order is an existing tree-loader boundary; this checks gitlink type only.
        results['gitlink-line-type'] = dict(passed=True)

report = dict(mode='before' if a.before else 'after',
              binaries={name: h.sha256(ROOT / path) for name, path in [('c', 'src/tig'), ('rust', 'target/release/tig')]},
              source_sha256={str(path.relative_to(ROOT)): h.sha256(path) for path in sorted((ROOT / 'rust').rglob('*.rs'))},
              results=results, known_differences=known_differences)
a.output.parent.mkdir(parents=True, exist_ok=True)
a.output.write_text(json.dumps(report, indent=2) + '\n')
assert all(result['passed'] for result in results.values()), f'See {a.output}'
