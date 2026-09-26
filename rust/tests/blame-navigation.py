#!/usr/bin/env python3
"""Blame navigation across renames and revision bounds, using a real PTY.

Run after cargo build --locked --release. Original upstream scripts are separate.
"""
import hashlib
import importlib.util
import json
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]
BINARY = ROOT / 'target/release/tig'
spec = importlib.util.spec_from_file_location('upstream', ROOT / 'rust/tests/upstream-suite.py')
upstream = importlib.util.module_from_spec(spec)
spec.loader.exec_module(upstream)


def main():
    checks = []
    with tempfile.TemporaryDirectory(prefix='tig-blame-navigation-') as temporary:
        repo = Path(temporary)
        env = upstream.environment([BINARY.parent])
        env.update(GIT_CONFIG_GLOBAL='/dev/null', TIGRC_SYSTEM='', TERM='xterm-256color',
                   TIGRC_USER=str(repo / 'tigrc'), TIG_SCRIPT=str(repo / 'steps'),
                   LINES='30', COLUMNS='200', HOME=str(repo))
        (repo / 'tigrc').write_text('set line-graphics = ascii\nset blame-view = line-number:yes,interval=1 text\n')

        def git(*args):
            return subprocess.check_output(['git', *args], cwd=repo, env=env).decode().strip()

        def screen(args, steps, output='screen'):
            target = repo / output
            (repo / 'steps').write_text(steps + f'\n:save-display {target}\n')
            code, timeout, transcript = upstream.terminal(
                [str(BINARY), '-C', str(repo), *args], env, 10)
            assert code == 0 and not timeout, transcript
            return target.read_text()

        git('init', '-q')
        git('config', 'user.name', 'Blame Test')
        git('config', 'user.email', 'blame@example.invalid')
        old, new = 'old\tname', 'new name'
        content = [f'line {i}' for i in range(1, 41)]
        (repo / old).write_text('\n'.join(content) + '\n')
        git('add', '--', old)
        git('commit', '-qm', 'original')
        original = git('rev-parse', 'HEAD')
        git('mv', old, new)
        content.insert(19, 'inserted')
        (repo / new).write_text('\n'.join(content) + '\n')
        git('commit', '-qam', 'rename and insert')
        renamed = git('rev-parse', 'HEAD')

        result = screen(['blame', new], ':21\n:view-blame')
        assert f'{original}:old    name' in result and 'line 20 of 40' in result, result
        checks.append('recursive blame uses historical quoted path and original line')
        result = screen(['blame', new], ':20\n:parent')
        assert f'{original}:old    name' in result and 'line 19 of 40' in result, result
        checks.append('parent follows previous filename and maps the inserted line')
        result = screen(['blame', new], ':21\n:view-blame\n:back')
        assert 'line 21 of 41' in result and ' 21| line 20' in result, result
        checks.append('back restores original blame position')
        result = screen(['blame', new], ':21\n:view-blame\n:parent')
        assert 'line 20 of 40' in result, result
        checks.append('parent of a root line leaves the view usable')

        content[-1] = 'middle change'
        (repo / new).write_text('\n'.join(content) + '\n')
        git('commit', '-qam', 'middle change')
        (repo / new).write_text('prefix\n' + '\n'.join(content) + '\n')
        git('commit', '-qam', 'later insertion')
        result = screen(['blame', f'{renamed}..HEAD', new], ':1\n:parent')
        assert f'{renamed}:{new}' in result and 'line 1 of 41' in result, result
        checks.append('parent preserves the blame revision lower bound')
        assert not git('status', '--porcelain', '--untracked-files=no'), 'navigation changed tracked files'
        checks.append('read-only navigation preserves index and worktree')
    evidence = {'scope': 'Additional real-PTY rename/boundary regressions; not full parity',
                'binary_sha256': hashlib.sha256(BINARY.read_bytes()).hexdigest(),
                'checks': checks}
    (ROOT / 'migration/evidence/blame-navigation-pty.json').write_text(json.dumps(evidence, indent=2) + '\n')
    print(json.dumps(evidence, indent=2))


if __name__ == '__main__':
    main()
