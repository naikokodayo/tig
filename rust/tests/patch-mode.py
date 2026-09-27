#!/usr/bin/env python3
"""Public C/Rust mode-plus-text selection regression, using the real Git index."""
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
    args.output.unlink(missing_ok=True)
    report = {'checks': [], 'source_sha256': {name: upstream.sha256(ROOT / name)
              for name in ('rust/patch.rs', 'rust/main.rs', 'rust/git.rs', 'src/stage.c', 'rust/tests/patch-mode.py')}}
    for binary in (ROOT / 'src/tig', ROOT / 'target/release/tig'):
        for reverse in (False, True):
            for action in ('stage-update-line', 'stage-update-part', 'status-update'):
                with tempfile.TemporaryDirectory(prefix='tig-mode-') as temporary:
                    home = Path(temporary)
                    repo = home / 'repo'
                    repo.mkdir()
                    env = upstream.environment([binary.parent])
                    env.update(HOME=str(home), TIGRC_SYSTEM='', TIGRC_USER='/dev/null', TERM='xterm')
                    def git(*argv):
                        return subprocess.check_output(['git', *argv], cwd=repo, env=env, stderr=subprocess.STDOUT)
                    git('init', '-q')
                    git('config', 'user.name', 'Mode Fixture')
                    git('config', 'user.email', 'mode@example.invalid')
                    git('config', 'core.filemode', 'true')
                    original = b'a\n1\n2\n3\n4\n5\n6\n7\n8\n9\n'
                    working = b'a\nselected\n1\n2\n3\n4\n5\n6\n7\n8\nother\n9\n'
                    path = repo / 'a'
                    path.write_bytes(original)
                    path.chmod(0o644)
                    git('add', 'a')
                    git('commit', '-qm', 'base')
                    path.write_bytes(working)
                    path.chmod(0o755)
                    if reverse:
                        git('add', 'a')
                    screen = home / 'screen'
                    script = home / 'script'
                    # Search the changed line, independent of diff header row counts.
                    script.write_text(f':{3 if reverse else 5}\n:enter\n:maximize\n:/selected\n:{action}\n:save-display {screen}\n:quit\n')
                    env['TIG_SCRIPT'] = str(script)
                    code, timeout, transcript = upstream.terminal([str(binary), '-C', str(repo), 'status'], env, 15)
                    expected = working.replace(b'selected\n', b'') if reverse else original.replace(b'a\n', b'a\nselected\n')
                    index = git('show', ':a')
                    mode = git('ls-files', '--stage').split()[0].decode()
                    safe = path.read_bytes() == working and path.stat().st_mode & 0o777 == 0o755
                    passed = code == 0 and not timeout and index == expected and mode == ('100644' if reverse else '100755') and safe
                    report['checks'].append(dict(binary=str(binary), binary_sha256=upstream.sha256(binary),
                        reverse=reverse, action=action, exit_code=code, timed_out=timeout,
                        index=index.decode(), mode=mode, worktree_preserved=safe, passed=passed,
                        screen=upstream.read(screen) if not passed else '', transcript=transcript if not passed else ''))
    args.output.write_text(json.dumps(report, indent=2) + '\n')
    assert all(check['passed'] for check in report['checks']), str(args.output)
    print('12 C/Rust mode-plus-text index/worktree checks passed')


if __name__ == '__main__':
    main()
