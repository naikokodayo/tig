#!/usr/bin/env python3
"""C/Rust show pathspec regression with a real Git repo and controlling PTY."""
import importlib.util
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location('upstream', ROOT / 'rust/tests/upstream-suite.py')
harness = importlib.util.module_from_spec(spec)
spec.loader.exec_module(harness)

with tempfile.TemporaryDirectory(prefix='tig-show-pathspec-') as temp:
    home = Path(temp)
    repo = home / 'repo'
    repo.mkdir()
    env = harness.environment([])
    env.update(HOME=str(home), GIT_CONFIG_GLOBAL='/dev/null', TIGRC_SYSTEM='',
               TIGRC_USER='/dev/null', TERM='xterm', LINES='40', COLUMNS='120')

    def git(*args):
        return subprocess.check_output(['git', *args], cwd=repo, env=env)

    git('init', '-q')
    git('config', 'user.name', 'Show Fixture')
    git('config', 'user.email', 'show@example.invalid')
    files = ['wanted.txt', 'other.txt', '--dash.txt', 'semi;touch PWN.txt', 'old name.txt']
    for name in files:
        (repo / name).write_text(name + ' base\n' + 'common\n' * 8)
    git('add', '--', *files)
    git('-c', 'commit.gpgsign=false', 'commit', '-qm', 'base')
    for name in files[:-1]:
        (repo / name).write_text(name + ' HEAD\n' + 'common\n' * 8)
    git('mv', '--', 'old name.txt', 'renamed name.txt')
    (repo / 'renamed name.txt').write_text('renamed name.txt HEAD\n' + 'common\n' * 8)
    git('add', '--all')
    git('-c', 'commit.gpgsign=false', 'commit', '-qm', 'change files')
    (repo / 'other.txt').write_text('other staged\n')
    git('add', '--', 'other.txt')
    (repo / 'wanted.txt').write_text('wanted unstaged\n')
    tracked = [repo / name for name in (*files[:-1], 'renamed name.txt')]
    index = git('write-tree')
    status = git('status', '--porcelain=v1', '-z')
    worktree = {path: path.read_bytes() for path in tracked}
    cases = [
        ('one', ['wanted.txt'], ['wanted.txt', '+wanted.txt HEAD'],
         ['other.txt', '--dash.txt', 'semi;touch PWN.txt', 'renamed name.txt']),
        ('multiple', ['wanted.txt', 'other.txt'], ['wanted.txt', 'other.txt'],
         ['--dash.txt', 'semi;touch PWN.txt', 'renamed name.txt']),
        ('option-like', ['--dash.txt'], ['--dash.txt', '+--dash.txt HEAD'],
         ['wanted.txt', 'other.txt', 'semi;touch PWN.txt', 'renamed name.txt']),
        ('renamed space', ['renamed name.txt'], ['renamed name.txt'],
         ['wanted.txt', 'other.txt', '--dash.txt', 'semi;touch PWN.txt']),
        ('shell metachar', ['semi;touch PWN.txt'], ['semi;touch PWN.txt'],
         ['wanted.txt', 'other.txt', '--dash.txt', 'renamed name.txt']),
        ('dot pathspec', ['.'], ['wanted.txt', 'other.txt', '--dash.txt',
                                'semi;touch PWN.txt', 'renamed name.txt'], []),
    ]
    for binary in (ROOT / 'src/tig', ROOT / 'target/release/tig'):
        for name, paths, included, excluded in cases:
            output = home / 'view.txt'
            output.unlink(missing_ok=True)
            script = home / 'steps'
            script.write_text((':refresh\n' if name == 'one' else '')
                              + f':save-view {output}\n:quit\n')
            code, timeout, transcript = harness.terminal(
                [str(binary), '-C', str(repo), 'show', 'HEAD', '--', *paths],
                {**env, 'TIG_SCRIPT': str(script)}, 15)
            view = output.read_text() if output.exists() else transcript
            assert code == 0 and not timeout, (binary, name, code, timeout, transcript)
            assert all(text in view for text in included), (binary, name, view)
            assert all(text not in view for text in excluded), (binary, name, view)
            assert git('write-tree') == index and git('status', '--porcelain=v1', '-z') == status
            assert all(path.read_bytes() == data for path, data in worktree.items())
            assert not (repo / 'PWN.txt').exists()
            print(f'{binary.parent.name}: {name}: pass', flush=True)
