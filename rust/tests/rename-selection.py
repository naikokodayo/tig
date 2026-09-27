#!/usr/bin/env python3
"""C/Rust text-rename selection parity using both real index paths."""
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
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    args.output.unlink(missing_ok=True)
    report = {'checks': [], 'source_sha256': {name: upstream.sha256(ROOT / name)
              for name in ('rust/patch.rs', 'rust/main.rs', 'rust/git.rs', 'src/stage.c', 'rust/tests/rename-selection.py')}}
    for binary in (ROOT / 'src/tig', ROOT / 'target/release/tig'):
        for mode in (0o644, 0o755):
            for action in ('stage-update-line', 'stage-update-part'):
                with tempfile.TemporaryDirectory(prefix='tig-rename-') as temporary:
                    home = Path(temporary)
                    repo = home / 'repo'
                    repo.mkdir()
                    env = upstream.environment([binary.parent])
                    env.update(HOME=str(home), TIGRC_SYSTEM='', TIGRC_USER='/dev/null', TERM='xterm')
                    def git(*argv):
                        return subprocess.check_output(['git', *argv], cwd=repo, env=env, stderr=subprocess.STDOUT)
                    git('init', '-q')
                    git('config', 'user.name', 'Rename Fixture')
                    git('config', 'user.email', 'rename@example.invalid')
                    git('config', 'core.filemode', 'true')
                    git('config', 'diff.renames', 'true')
                    old, new = repo / 'old name', repo / 'new name'
                    original = b'first\n1\n2\n3\n4\n5\n6\n7\n8\nlast\n'
                    working = original.replace(b'first\n', b'first\nselected\npair\n').replace(b'last\n', b'remaining\nlast\n')
                    old.write_bytes(original)
                    old.chmod(mode)
                    git('add', '--', old.name)
                    git('commit', '-qm', 'base')
                    old.rename(new)
                    new.write_bytes(working)
                    git('add', '-A')
                    assert b'rename from old name\nrename to new name\n' in git('diff', '--cached')
                    script = home / 'script'
                    # Aggregate stage view preserves both rename paths in its diff.
                    script.write_text(f':2\n:enter\n:maximize\n:/[+]selected\n:{action}\n:quit\n')
                    env['TIG_SCRIPT'] = str(script)
                    code, timeout, transcript = upstream.terminal([str(binary), '-C', str(repo), 'status'], env, 15)
                    entries = git('ls-files', '--stage', '-z')
                    blob = subprocess.run(['git', 'show', ':old name'], cwd=repo, env=env, capture_output=True)
                    safe = not old.exists() and new.read_bytes() == working and new.stat().st_mode & 0o777 == mode
                    passed = (code == 0 and not timeout and blob.returncode == 0
                              and blob.stdout == working.replace(b'selected\npair\n' if action == 'stage-update-part' else b'selected\n', b'')
                              and git('ls-files', '-z') == b'old name\0'
                              and entries.startswith(f'100{mode:o} '.encode()) and safe)
                    report['checks'].append(dict(binary=str(binary), binary_sha256=upstream.sha256(binary),
                        mode=mode, action=action, exit_code=code, timed_out=timeout,
                        index=entries.decode(), old_path_content=blob.stdout.decode(),
                        worktree_preserved=safe, passed=passed, transcript=transcript if not passed else ''))
    args.output.write_text(json.dumps(report, indent=2) + '\n')
    assert all(c['passed'] for c in report['checks']), str(args.output)
    print('8 C/Rust rename line/block index and worktree checks passed')


if __name__ == '__main__':
    main()
