#!/usr/bin/env python3
"""Real Git/PTY blame -> worktree diff, historical blob and unfiltered main."""
import importlib.util
import os
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location('upstream', ROOT / 'rust/tests/upstream-suite.py')
harness = importlib.util.module_from_spec(spec)
spec.loader.exec_module(harness)
checks = 0
with tempfile.TemporaryDirectory(prefix='tig-blame-context-') as temporary:
    repo = Path(temporary)
    env = harness.environment([])
    env.update(GIT_CONFIG_GLOBAL='/dev/null', TIGRC_SYSTEM='', LC_ALL='C',
               TIGRC_USER=str(repo / 'tigrc'), TIG_SCRIPT=str(repo / 'steps'),
               COLUMNS='160', LINES='40')
    (repo / 'tigrc').write_text('set vertical-split = no\nset line-graphics = ascii\n'
                               'set blame-view = line-number:yes,interval=1 text\n')

    def git(*args):
        return subprocess.check_output(['git', '-C', str(repo), *args], env=env)

    git('init', '-q')
    git('config', 'user.name', 'Test')
    git('config', 'user.email', 'test@example.invalid')
    old, new = 'old name.txt', 'new name.txt'
    (repo / old).write_text('original line\nsecond line\nthird line\n')
    git('add', '--', old)
    git('-c', 'commit.gpgsign=false', 'commit', '-qm', 'original')
    original = git('rev-parse', 'HEAD').decode().strip()
    git('mv', old, new)
    git('-c', 'commit.gpgsign=false', 'commit', '-qm', 'rename')
    renamed = git('rev-parse', 'HEAD').decode().strip()
    (repo / 'unrelated').write_text('unrelated content\n')
    git('add', 'unrelated')
    git('-c', 'commit.gpgsign=false', 'commit', '-qm', 'unrelated tip')

    def check(label, args, setup, action, expected, view, modes=('c', 'rust')):
        global checks
        before = (git('diff', '--binary'), git('diff', '--cached', '--binary'))
        for mode in modes:
            binary = Path(os.environ.get('TIG_CONTEXT_RUST', ROOT / 'target/release/tig')) if mode == 'rust' else ROOT / 'src/tig'
            paths = [repo / name for name in ('before', 'opened', 'refreshed', 'closed')]
            for path in paths:
                path.unlink(missing_ok=True)
            steps = setup + f'\n:save-display {paths[0]}\n:{action}\n'
            if action == 'enter':
                steps += ':maximize\n'
            steps += (f':save-display {paths[1]}\n:refresh\n:save-display {paths[2]}\n'
                      f':view-close\n:save-display {paths[3]}\n:quit\n')
            (repo / 'steps').write_text(steps)
            code, timeout, transcript = harness.terminal([str(binary), '-C', str(repo), *args], env, 10)
            assert code == 0 and not timeout, (label, mode, transcript)
            opened, refreshed, closed = [p.read_text() for p in paths[1:]]
            screens = (opened, refreshed)
            if mode == 'c' and view == 'diff':
                # C refresh discards the blame argv and shows HEAD instead.
                assert '[diff]' in refreshed and 'unrelated tip' in refreshed, refreshed
                print(f'C exception: {label} refresh reloads HEAD')
                screens = (opened,)
                if label == 'null commit without parent':
                    assert "unknown option `encoding=UTF-8'" in opened, opened
                    print('C exception: no-parent diff rejects --encoding=UTF-8')
                    screens = ()
            for screen in screens:
                assert f'[{view}]' in screen, (label, mode, screen)
                for text in expected:
                    assert text in screen, (label, mode, text, screen)
                if view == 'diff':
                    assert 'unrelated' not in screen, (label, mode, screen)
            assert closed == paths[0].read_text(), (label, mode, 'close changed blame', closed, paths[0].read_text())
            checks += 1
        assert before == (git('diff', '--binary'), git('diff', '--cached', '--binary'))
        print(f'PASS {label} ({", ".join(modes)})')

    check('rename historical blob', ['blame', new], ':2', 'view-blob',
          [old, 'original line', 'second line', 'third line'], 'blob')
    check('rename selects original commit in main', ['blame', new], ':2', 'view-main',
          ['original', 'rename', 'unrelated tip', original], 'main')
    check('main clears inherited revision and file filters', [renamed, '--', new],
          ':view-tree\n:/new name\n:view-blob\n:2\n:view-blame', 'view-main',
          ['original', 'rename', 'unrelated tip', original], 'main')
    (repo / new).write_text('original line\nworktree replacement\nthird line\n')
    (repo / 'unrelated').write_text('other dirty file\n')
    check('null commit blob uses HEAD', ['blame', new], ':2', 'view-blob',
          [new, 'original line', 'second line', 'third line'], 'blob')
    check('null commit diff', ['blame', new], ':2', 'enter',
          [f'diff --git a/{new} b/{new}', '-second line', '+worktree replacement'], 'diff')
    (repo / 'added file').write_text('new worktree line\n')
    git('add', '--', 'added file')
    check('null commit without parent', ['blame', 'added file'], ':1', 'enter',
          ['new file mode', '+new worktree line'], 'diff')
print(f'{checks} navigation checks passed; tracked index/worktree unchanged')
