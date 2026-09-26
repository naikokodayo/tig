#!/usr/bin/env python3
"""Real C/Rust index checks for status filters across aggregate stage and refresh."""
import argparse
import hashlib
import importlib.util
import json
import subprocess
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location('upstream', ROOT / 'rust/tests/upstream-suite.py')
upstream = importlib.util.module_from_spec(spec)
spec.loader.exec_module(upstream)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    report = {'source_commit': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
              'scope': 'Real Git index/worktree boundary checks, not original suite receipts', 'checks': []}
    for binary in (ROOT / 'src/tig', ROOT / 'target/release/tig'):
        for case in ('stage', 'stage-refresh', 'unstage-refresh', 'multiple-paths', 'filter-off',
                     'file-stage', 'file-unstage', 'untracked-open'):
            with tempfile.TemporaryDirectory(prefix='tig-status-filter-') as temporary:
                home = Path(temporary)
                repo = home / 'repo'
                repo.mkdir()
                (repo / 'sub').mkdir()
                env = upstream.environment([binary.parent])
                env.update(HOME=str(home), TIGRC_SYSTEM='', TIGRC_USER='/dev/null', TERM='xterm')
                def git(*argv):
                    return subprocess.check_output(['git', *argv], cwd=repo, env=env, stderr=subprocess.STDOUT)
                git('init', '-q')
                git('config', 'user.name', 'Filter Fixture')
                git('config', 'user.email', 'filter@example.invalid')
                files = ['outside', 'sub/file', 'sub/more space']
                for file in files:
                    (repo / file).write_text('base\n')
                git('add', '.')
                git('commit', '-qm', 'base')
                for file in files:
                    (repo / file).write_text('changed\n')
                (repo / 'sentinel-staged').write_text('outside staged sentinel\n')
                (repo / 'sentinel-untracked').write_text('outside untracked sentinel\n')
                (repo / 'sub/untracked').write_text('inside untracked\n')
                git('add', 'sentinel-staged')
                unstage = case in ('unstage-refresh', 'file-unstage')
                if unstage:
                    git('add', *files)
                sentinel = git('ls-files', '--stage', '-z', '--', 'sentinel-staged', 'sentinel-untracked')
                outside = git('show', ':outside')
                cwd, filters = repo, ['sub']
                if case == 'multiple-paths':
                    cwd, filters = repo, files[1:]
                script = home / 'script'
                commands = (':3\n:status-update\n' if case == 'file-unstage' else
                            ':5\n:status-update\n' if case == 'file-stage' else
                            ':8\n:enter\n:status-update\n' if case == 'untracked-open' else None)
                script.write_text((commands + ':quit\n') if commands else ((':toggle file-filter\n' if case == 'filter-off' else '') +
                                  (':2\n' if unstage else ':4\n') + ':enter\n' +
                                  (':refresh\n' if 'refresh' in case else '') +
                                  ':1\n:status-update\n:quit\n'))
                env['TIG_SCRIPT'] = str(script)
                code, timeout, transcript = upstream.terminal([str(binary), '-C', str(cwd), 'status', *filters], env, 15)
                actual = sorted(x.decode() for x in git('diff', '--cached', '--name-only', '-z').split(b'\0') if x)
                expected = (['outside', 'sub/more space'] if case == 'file-unstage' else
                            ['sub/file'] if case == 'file-stage' else
                            ['sub/untracked'] if case == 'untracked-open' else
                            ['outside'] if unstage else files if case == 'filter-off' else files[1:])
                expected = sorted([*expected, 'sentinel-staged'])
                index_safe = case == 'filter-off' or git('show', ':outside') == outside
                sentinels_safe = (git('ls-files', '--stage', '-z', '--', 'sentinel-staged', 'sentinel-untracked') == sentinel
                                  and (repo / 'sentinel-staged').read_bytes() == b'outside staged sentinel\n'
                                  and (repo / 'sentinel-untracked').read_bytes() == b'outside untracked sentinel\n'
                                  and (repo / 'sub/untracked').read_bytes() == b'inside untracked\n')
                worktree_safe = all((repo / file).read_bytes() == b'changed\n' for file in files)
                passed = code == 0 and not timeout and actual == expected and index_safe and worktree_safe and sentinels_safe
                report['checks'].append({'binary': str(binary), 'binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(),
                                         'case': case, 'exit_code': code, 'timed_out': timeout,
                                         'cached_paths': actual, 'expected_cached_paths': expected,
                                         'outside_index_preserved': index_safe, 'sentinels_preserved': sentinels_safe, 'worktree_preserved': worktree_safe,
                                         'passed': passed, 'transcript': transcript if not passed else ''})
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2) + '\n')
    assert all(check['passed'] for check in report['checks']), str(args.output)
    print(f"{len(report['checks'])} C/Rust filtered index checks passed")


if __name__ == '__main__':
    main()
