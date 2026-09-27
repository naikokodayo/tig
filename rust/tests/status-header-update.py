#!/usr/bin/env python3
"""Compare C and Rust status-header updates against real Git indexes in a PTY."""
import importlib.util
import os
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location('upstream', ROOT / 'rust/tests/upstream-suite.py')
upstream = importlib.util.module_from_spec(spec)
spec.loader.exec_module(upstream)


def run(binary, case):
    with tempfile.TemporaryDirectory(prefix='tig-header-') as temporary:
        home = Path(temporary)
        repo = home / 'repo'
        repo.mkdir()
        env = upstream.environment([binary.parent])
        env.update(HOME=str(home), TIGRC_SYSTEM='', TIGRC_USER='/dev/null', TERM='xterm')

        def git(*args):
            return subprocess.check_output(['git', *args], cwd=repo, env=env, stderr=subprocess.STDOUT)

        git('init', '-q')
        git('config', 'user.name', 'Header Fixture')
        git('config', 'user.email', 'header@example.invalid')
        tracked = ['a space', '-dash', 'outside', 'old name', 'sub/a', 'sub/b']
        (repo / 'sub').mkdir()
        for name in tracked:
            (repo / name).write_text('base\n')
        git('add', '.')
        git('commit', '-qm', 'base')
        selection, filters, expected = ':4', [], []
        if case in ('unstaged', 'staged'):
            for name in tracked[:2]:
                (repo / name).write_text('changed\n')
            expected = tracked[:2]
            if case == 'staged':
                (repo / 'old name').rename(repo / 'new name')
                git('add', '-A')
                # C restores only the old path; Rust also clears the new path.
                selection = ':2'
                expected = ['new name'] if binary == ROOT / 'src/tig' else []
        elif case == 'untracked':
            names = ['new space', ':(glob)*']
            if sys.platform == 'linux':
                names.append(os.fsdecode(b'raw-\xff'))
            for name in names:
                (repo / name).write_text('new\n')
            selection, expected = ':6', names
        elif case == 'filtered':
            for name in ('outside', 'sub/a', 'sub/b'):
                (repo / name).write_text('changed\n')
            filters, expected = ['sub'], ['sub/a', 'sub/b']
        elif case != 'empty':
            raise ValueError(case)
        before = {str(path.relative_to(repo)): path.read_bytes()
                  for path in repo.rglob('*') if path.is_file() and '.git' not in path.parts}
        script = home / 'script'
        script.write_text(f'{selection}\n:status-update\n:quit\n')
        env['TIG_SCRIPT'] = str(script)
        code, timeout, transcript = upstream.terminal(
            [str(binary), '-C', str(repo), 'status', *filters], env, 15)
        actual = sorted(os.fsdecode(name) for name in
                        git('diff', '--cached', '--name-only', '-z').split(b'\0') if name)
        after = {str(path.relative_to(repo)): path.read_bytes()
                 for path in repo.rglob('*') if path.is_file() and '.git' not in path.parts}
        assert code == 0 and not timeout and actual == sorted(expected) and before == after, (
            binary, case, code, timeout, actual, expected, before == after, transcript[-1000:])
        for name in tracked:
            assert git('show', f':{name}') == (before[name] if name in expected else b'base\n')
        for name in expected:
            assert git('show', f':{name}') == before[name]


if __name__ == '__main__':
    for program in ('src/tig', 'target/release/tig'):
        for scenario in ('unstaged', 'staged', 'untracked', 'filtered', 'empty'):
            run(ROOT / program, scenario)
            print(f'PASS {program} {scenario}')
