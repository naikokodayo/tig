#!/usr/bin/env python3
"""Focused real Git/PTY stage, blob and status -> blame regressions."""
import importlib.util
from pathlib import Path
import re
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location('upstream', ROOT / 'rust/tests/upstream-suite.py')
harness = importlib.util.module_from_spec(spec)
spec.loader.exec_module(harness)
checks = 0
with tempfile.TemporaryDirectory(prefix='tig-file-blame-') as temporary:
    directory = Path(temporary)
    repo = directory / 'repo'
    repo.mkdir()
    env = harness.environment([])
    env.update(GIT_CONFIG_GLOBAL='/dev/null', TIGRC_SYSTEM='', LC_ALL='C',
               TIGRC_USER=str(directory / 'tigrc'), TIG_SCRIPT=str(directory / 'steps'),
               COLUMNS='100', LINES='30')
    (directory / 'tigrc').write_text('set vertical-split = no\nset line-graphics = ascii\n'
                                   'set blame-view = line-number:yes,interval=1 text\n')
    def git(*args):
        return subprocess.check_output(['git', '-C', str(repo), *args], env=env)
    git('init', '-q')
    git('config', 'user.name', 'Test')
    git('config', 'user.email', 'test@example.invalid')
    name = 'file name.txt'
    path = repo / name
    lines = [f'line {i}\n' for i in range(1, 61)]
    path.write_text(''.join(lines))
    git('add', '--', name)
    git('-c', 'commit.gpgsign=false', 'commit', '-qm', 'base')
    base = git('rev-parse', 'HEAD').decode().strip()

    def check(label, args, steps, expected, content, back_view, modes=('c', 'rust')):
        global checks
        before = (git('diff', '--binary'), git('diff', '--cached', '--binary'))
        for mode in modes:
            binary = ROOT / ('src/tig' if mode == 'c' else 'target/release/tig')
            screens = [directory / n for n in ('opened', 'refresh', 'back')]
            for screen in screens:
                screen.unlink(missing_ok=True)
            before_screen = directory / 'before'
            before_screen.unlink(missing_ok=True)
            (directory / 'steps').write_text(steps + f'\n:save-display {before_screen}\n:view-blame\n'
                f':save-display {screens[0]}\n:refresh\n:save-display {screens[1]}\n'
                f':view-close\n:save-display {screens[2]}\n:quit\n')
            code, timeout, transcript = harness.terminal([str(binary), '-C', str(repo), *args], env, 10)
            assert code == 0 and not timeout, (label, mode, transcript)
            for screen in screens[:2]:
                text = screen.read_text()
                assert '[blame]' in text and f'line {expected} of ' in text, (label, mode, text)
                assert f'{expected}| {content}' in text, (label, mode, text)
            assert f'[{back_view}]' in screens[2].read_text(), (label, mode, screens[2].read_text())
            position = re.search(r'line \d+ of \d+', before_screen.read_text())
            if position:
                assert position.group() in screens[2].read_text(), (label, mode, 'back lost position')
            checks += 1
        assert before == (git('diff', '--binary'), git('diff', '--cached', '--binary'))
        print(f'PASS {label} ({", ".join(modes)})')

    path.write_text('prefix\n' + ''.join(lines))
    git('-c', 'commit.gpgsign=false', 'commit', '-qam', 'shift')
    check('historical blob', [base], ':view-tree\n:/file name\n:view-blob\n:43', 43, 'line 43', 'blob')
    path.write_text('worktree\n' + path.read_text())
    check('status worktree', ['status'], ':5', 1, 'worktree', 'status')
    check('blob from worktree grep', ['grep', 'line 43', '--', name], ':2\n:enter\n:maximize', 45, 'line 43', 'blob')
    check('stage stat', ['status'], ':5\n:enter\n:maximize', 1, 'worktree', 'stage')
    check('stage addition', ['status'], ':5\n:enter\n:maximize\n:/worktree', 1, 'worktree', 'stage')
    git('reset', '--hard', base)
    path.write_text(''.join(lines[:19] + lines[20:]))
    check('unstaged deletion', ['status'], ':5\n:enter\n:maximize\n:/-line 20', 20, 'line 20', 'stage')
    git('add', '--', name)
    check('staged deletion', ['status'], ':3\n:enter\n:maximize\n:/-line 20', 20, 'line 20', 'stage')
    git('reset', '--hard', base)
    path.write_text('staged prefix\n' + ''.join(lines))
    git('add', '--', name)
    path.write_text('staged prefix\n' + ''.join(lines[:19] + lines[20:]))
    check('deletion after staged insertion', ['status'], ':5\n:enter\n:maximize\n:/-line 20', 20, 'line 20', 'stage')
    git('reset', '--hard', base)
    git('mv', name, 'renamed file.txt')
    path = repo / 'renamed file.txt'
    path.write_text(''.join(lines[:19] + lines[20:]))
    check('deletion after staged rename', ['status'], ':5\n:enter\n:maximize\n:/-line 20', 20, 'line 20', 'stage')
    git('config', 'status.renames', 'false')
    check('rename detection disabled in status config', ['status'], ':5\n:enter\n:maximize\n:/-line 20', 20, 'line 20', 'stage')
    git('config', '--unset', 'status.renames')
    def reject(label, steps, view, modes=('c', 'rust'), args=('status',)):
        global checks
        before = (git('diff', '--binary'), git('diff', '--cached', '--binary'))
        for mode in modes:
            binary = ROOT / ('src/tig' if mode == 'c' else 'target/release/tig')
            screen = directory / 'rejected'
            screen.unlink(missing_ok=True)
            (directory / 'steps').write_text(steps + f'\n:view-blame\n:save-display {screen}\n:quit\n')
            code, timeout, transcript = harness.terminal([str(binary), '-C', str(repo), *args], env, 10)
            assert code == 0 and not timeout, (label, mode, transcript)
            text = screen.read_text()
            assert f'[{view}]' in text and '[blame]' not in text, (label, mode, text)
            checks += 1
        assert before == (git('diff', '--binary'), git('diff', '--cached', '--binary'))
        print(f'PASS {label} ({", ".join(modes)})')

    git('reset', '--hard', base)
    lines[19] = '-- marker\n'
    path = repo / name
    path.write_text(''.join(lines))
    git('-c', 'commit.gpgsign=false', 'commit', '-qam', 'header-like content')
    path.write_text(''.join(lines[:19] + lines[20:]))
    check('header-like deleted text', ['status'], ':5\n:enter\n:maximize\n:/-- marker', 20, '-- marker', 'stage')
    git('reset', '--hard', base)
    lines[19] = 'line 20\n'
    reject('status heading', ':2', 'status')
    (repo / 'untracked.txt').write_text('untracked\n')
    reject('untracked status', ':7', 'status')
    reject('untracked stage', ':7\n:enter\n:maximize', 'stage')
    path = repo / name
    path.write_text('staged only\n' + ''.join(lines))
    git('add', '--', name)
    path.write_text(''.join(lines))
    reject('cached blob rejects worktree attribution', ':2\n:enter\n:maximize', 'blob',
           ('rust',), ('grep', '--cached', 'line 43', '--', name))
    # C maps this never-committed deletion to a neighboring HEAD line. Fail closed.
    check('C baseline: index-only deletion selects unrelated HEAD line', ['status'],
          ':5\n:enter\n:maximize\n:/-staged only', 1, 'line 1', 'stage', ('c',))
    reject('deleted index-only addition', ':5\n:enter\n:maximize\n:/-staged only', 'stage', ('rust',))
print(f'{checks} navigation checks passed; tracked index/worktree unchanged')
