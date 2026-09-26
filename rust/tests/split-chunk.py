#!/usr/bin/env python3
"""C/Rust split display parity and real index/worktree preservation."""
import argparse
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
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', required=True, type=Path)
    args = parser.parse_args()
    report = {'checks': []}
    for binary in (ROOT / 'src/tig', ROOT / 'target/release/tig'):
        for newline in (False, True):
            for staged in (False, True):
                with tempfile.TemporaryDirectory(prefix='tig-split-') as temporary:
                    home = Path(temporary)
                    repo = home / 'repo'
                    repo.mkdir()
                    env = upstream.environment([binary.parent])
                    env.update(HOME=str(home), TIGRC_SYSTEM='', TIGRC_USER='/dev/null', TERM='xterm')
                    def git(*argv):
                        return subprocess.check_output(['git', *argv], cwd=repo, env=env, stderr=subprocess.STDOUT)
                    git('init', '-q')
                    git('config', 'user.name', 'Split Fixture')
                    git('config', 'user.email', 'split@example.invalid')
                    ending = b'\n' if newline else b''
                    (repo / 'a').write_bytes(b'a\n1\n2\n3\n4\n5\n6\n7\n8\n9\n10' + ending)
                    git('add', 'a')
                    git('commit', '-qm', 'base')
                    working = b'a CHANGED\n1\n2\nedited-too\n4\n5\nedited-too\n7\n8' + ending
                    (repo / 'a').write_bytes(working)
                    if staged:
                        git('add', 'a')
                    before = git('ls-files', '--stage', '-z')
                    screen = home / 'split.screen'
                    script = home / 'script'
                    script.write_text(f':{3 if staged else 5}\n:enter\n:maximize\n:12\n:stage-split-chunk\n:save-display {screen}\n:quit\n')
                    env['TIG_SCRIPT'] = str(script)
                    code, timeout, transcript = upstream.terminal([str(binary), '-C', str(repo), 'status'], env, 15)
                    headers = [row for row in upstream.read(screen).splitlines() if row.startswith('@@')]
                    expected = ['@@ -1,3 +1,3 @@', '@@ -2,5 +2,5 @@', '@@ -5,5 +5,5 @@' if newline else '@@ -5,4 +5,4 @@',
                                '@@ -8,4 +8,2 @@' if newline else '@@ -8,4 +8,1 @@']
                    index_safe = git('ls-files', '--stage', '-z') == before
                    worktree_safe = (repo / 'a').read_bytes() == working
                    passed = code == 0 and not timeout and headers == expected and index_safe and worktree_safe
                    report['checks'].append(dict(binary=str(binary), binary_sha256=upstream.sha256(binary),
                        newline=newline, staged=staged, exit_code=code, timed_out=timeout, headers=headers,
                        index_preserved=index_safe, worktree_preserved=worktree_safe, passed=passed,
                        transcript=transcript if not passed else ''))
    args.output.write_text(json.dumps(report, indent=2) + '\n')
    assert all(check['passed'] for check in report['checks']), str(args.output)
    print('8 C/Rust split display and index/worktree checks passed')


if __name__ == '__main__':
    main()
