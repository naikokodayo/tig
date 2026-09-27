#!/usr/bin/env python3
"""Focused real Git + paired C/Rust PTY blame option checks; prints one result."""
import importlib.util
import json
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location('upstream', ROOT / 'rust/tests/upstream-suite.py')
upstream = importlib.util.module_from_spec(spec)
spec.loader.exec_module(upstream)


def main():
    checks = []
    with tempfile.TemporaryDirectory(prefix='tig-blame-options-') as temporary:
        repo = Path(temporary)
        env = upstream.environment([])
        env.update(HOME=str(repo), GIT_CONFIG_GLOBAL='/dev/null', TIGRC_SYSTEM='',
                   TIGRC_USER=str(repo / 'tigrc'), TIG_SCRIPT=str(repo / 'steps'),
                   TERM='xterm-256color', LINES='30', COLUMNS='200')
        base_config = 'set line-graphics = ascii\nset blame-view = id:yes,width=7 file-name:auto text\n'

        def git(*args):
            return subprocess.check_output(['git', *args], cwd=repo, env=env).decode().strip()

        def screen(mode, args, settings='', steps='', success=True):
            binary = ROOT / ('src/tig' if mode == 'c' else 'target/release/tig')
            target = repo / 'screen'
            target.unlink(missing_ok=True)
            (repo / 'tigrc').write_text(base_config + settings)
            (repo / 'steps').write_text(steps + f'\n:save-display {target}\n:quit\n')
            code, timeout, transcript = upstream.terminal(
                [str(binary), '-C', str(repo), 'blame', *args], env, 10)
            if not success:
                assert code != 0 and not timeout and not target.exists(), transcript
                return transcript
            assert code == 0 and not timeout and target.exists(), transcript
            return target.read_text()

        def pair(name, args, settings='', steps=''):
            outputs = [screen(mode, args, settings, steps) for mode in ('c', 'rust')]
            # Status bars reflect distinct UI implementations; actual blame rows must match.
            bodies = [s.split('\n[blame]')[0].rstrip() for s in outputs]
            assert bodies[0] == bodies[1], (name, outputs)
            checks.append(name)
            return outputs[1]

        git('init', '-q')
        git('config', 'user.name', 'Options Test')
        git('config', 'user.email', 'options@example.invalid')
        content = ''.join(f'original substantial content line {i:02d}\n' for i in range(1, 9))
        (repo / 'source').write_text(content)
        git('add', 'source')
        git('commit', '-qm', 'source')
        root = git('rev-parse', 'HEAD')
        (repo / 'copy name').write_text(content)
        git('add', 'copy name')
        git('commit', '-qm', 'copy')
        copied = git('rev-parse', 'HEAD')
        (repo / 'copy name').write_text(content.replace('line 04', 'line FOUR'))
        git('commit', '-qam', 'edit')
        head = git('rev-parse', 'HEAD')
        result = pair('CLI repeated copy detection and auto filename', ['-C', '-C', 'HEAD', '--', 'copy name'])
        assert f'{root[:7]} source' in result and f'{head[:7]} copy name' in result
        result = pair('configured copy detection', ['HEAD', '--', 'copy name'], 'set blame-options = -C -C\n')
        assert 'source' in result
        result = pair('copy option displays single source filename', ['-C', 'HEAD', '--', 'source'])
        assert f'{root[:7]} source' in result
        pair('CLI options replace configured options', ['-w', 'HEAD', '--', 'copy name'], 'set blame-options = -C -C\n')
        result = pair('copied line navigation', ['-C', '-C', 'HEAD', '--', 'copy name'], steps=':view-blame')
        assert f'{root}:source' in result
        pair('two-dot revision bounds', [f'{copied}..HEAD', '--', 'copy name'])
        pair('explicit revision bounds', ['HEAD', f'^{copied}', '--', 'copy name'])
        pair('reverse CLI', ['--reverse', f'{copied}..HEAD', '--', 'copy name'])
        pair('reverse config', [f'{copied}..HEAD', '--', 'copy name'], 'set commit-order = reverse\n')
        pair('CLI order overrides reverse config', ['--topo-order', f'{copied}..HEAD', '--', 'copy name'], 'set commit-order = reverse\n')
        pair('age bound', ['--max-age=1', 'HEAD', '--', 'copy name'])
        pair('bounds retained when navigating parent', [f'{copied}..HEAD', '--', 'copy name'], steps=':4\n:parent')
        (repo / 'copy name').write_text(content.replace('line 04', 'line FOUR').replace('line 08', 'uncommitted'))
        result = pair('implicit revision includes worktree', ['copy name'])
        assert '0000000' in result and 'uncommitted' in result
        # Fail closed before any attacker-controlled options reach git blame.
        for args in [[], ['HEAD', root, '--', 'source'], ['HEAD...HEAD', '--', 'source'],
                     ['--output=sentinel', '--', 'source'], ['--contents=/etc/passwd', '--', 'source'],
                     ['--textconv', '--', 'source'], ['--rever', '--', 'source'],
                     ['--', '../escape'], ['--', 'source', 'copy name'],
                     ['-Cbad', '--', 'source'], ['--', '.git/config']]:
            screen('rust', args, success=False)
        assert not (repo / 'sentinel').exists()
        checks.append('unsafe options, ambiguous revisions and invalid paths fail closed')
        assert git('diff', '--name-only') == 'copy name'
        assert not git('diff', '--cached', '--name-only')
        checks.append('read-only operations preserve index and worktree')
    print(json.dumps({'checks': checks, 'passed': len(checks)}, indent=2))


if __name__ == '__main__':
    main()
