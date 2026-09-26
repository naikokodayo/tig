#!/usr/bin/env python3
"""Real Git/PTY check for reflog history and stash diff selection/return."""
import importlib.util
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location('upstream', ROOT / 'rust/tests/upstream-suite.py')
upstream = importlib.util.module_from_spec(spec)
spec.loader.exec_module(upstream)

with tempfile.TemporaryDirectory(prefix='tig-reflog-stash-') as temporary:
    repo = Path(temporary)
    env = upstream.environment([])
    env.update(GIT_AUTHOR_NAME='Test', GIT_AUTHOR_EMAIL='test@example.invalid',
               GIT_COMMITTER_NAME='Test', GIT_COMMITTER_EMAIL='test@example.invalid',
               HOME=str(repo), GIT_CONFIG_GLOBAL='/dev/null', TIGRC_SYSTEM='',
               TIGRC_USER='/dev/null', TIG_SCRIPT=str(repo / 'steps'))
    def git(*args):
        return subprocess.check_output(['git', '-C', str(repo), *args], env=env, text=True).strip()
    git('init', '-q')
    for subject in ('first', 'second'):
        (repo / 'file').write_text(subject + '\n')
        git('add', 'file')
        git('commit', '-qm', subject)
    old = git('rev-parse', 'HEAD')
    for subject in ('older', 'newer'):
        (repo / 'file').write_text(subject + '\n')
        git('stash', 'push', '-qm', subject)
    (repo / 'config').write_text('set show-changes = no\n')
    env['TIGRC_USER'] = str(repo / 'config')
    for name, reference, child, expected in (
            ('reflog', 'HEAD@{1}', 'main', old),
            ('stash', 'stash@{1}', 'diff', '+older')):
        (repo / 'steps').write_text(
            ':move-down\n' + f':save-display {repo / "selected"}\n'
            + ':enter\n' + f':save-display {repo / "child"}\n'
            + ':view-close\n:refresh\n' + f':save-display {repo / "returned"}\n:quit\n')
        code, timed_out, transcript = upstream.terminal(
            [str(ROOT / 'target/release/tig'), '-C', str(repo), name], env, 10)
        assert code == 0 and not timed_out, (name, code, transcript)
        selected = (repo / 'selected').read_text()
        screen = (repo / 'child').read_text()
        returned = (repo / 'returned').read_text()
        assert reference in selected, selected
        assert f'[{child}]' in screen and expected in screen, screen
        assert f'[{name}]' in returned and reference in returned, returned
        print(f'PASS: {name} selects older entry, opens {child}, returns and refreshes')
