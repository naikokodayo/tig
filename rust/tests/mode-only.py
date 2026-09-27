#!/usr/bin/env python3
"""Mode-only stage-view parity and stale-index refusal through public binaries."""
import argparse
import importlib.util
import json
from pathlib import Path
import shutil
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
              for name in ('rust/patch.rs', 'rust/main.rs', 'rust/git.rs', 'src/stage.c', 'rust/tests/mode-only.py')}}
    real_git = shutil.which('git')
    assert real_git
    for binary in (ROOT / 'src/tig', ROOT / 'target/release/tig'):
        for executable in (False, True):
            for reverse in (False, True):
                for stale in ((False,) if binary.parent.name == 'src' else (False, True)):
                    with tempfile.TemporaryDirectory(prefix='tig-mode-only-') as temporary:
                        home = Path(temporary)
                        repo = home / 'repo'
                        repo.mkdir()
                        env = upstream.environment([binary.parent])
                        env.update(HOME=str(home), TIGRC_SYSTEM='', TIGRC_USER='/dev/null', TERM='xterm')
                        def git(*argv):
                            return subprocess.check_output([real_git, *argv], cwd=repo, env=env, stderr=subprocess.STDOUT)
                        git('init', '-q')
                        git('config', 'user.name', 'Mode Fixture')
                        git('config', 'user.email', 'mode@example.invalid')
                        git('config', 'core.filemode', 'true')
                        path = repo / 'space name'
                        content = b'unchanged\n'
                        old_mode, new_mode = (0o755, 0o644) if executable else (0o644, 0o755)
                        path.write_bytes(content)
                        path.chmod(old_mode)
                        git('add', '--', path.name)
                        git('commit', '-qm', 'base')
                        path.chmod(new_mode)
                        if reverse:
                            git('add', '--', path.name)
                        expected_mode = old_mode if reverse else new_mode
                        if stale:
                            # Change the real index after the displayed diff was read,
                            # immediately before the first apply/check. No fake output.
                            shim = home / 'bin'
                            shim.mkdir()
                            wrapper = shim / 'git'
                            wrapper.write_text('#!/bin/sh\n'
                                'for arg do\n'
                                '  if [ "$arg" = apply ] && [ ! -f "$MODE_SNAPSHOT" ]; then\n'
                                '    "$REAL_GIT" update-index "$MODE_CHANGE" -- "space name" || exit 91\n'
                                '    cp .git/index "$MODE_SNAPSHOT" || exit 92\n'
                                '    break\n'
                                '  fi\n'
                                'done\nexec "$REAL_GIT" "$@"\n')
                            wrapper.chmod(0o755)
                            env.update(PATH=str(shim) + ':' + env['PATH'], REAL_GIT=real_git,
                                       MODE_CHANGE='--chmod=+x' if expected_mode == 0o755 else '--chmod=-x',
                                       MODE_SNAPSHOT=str(home / 'stale-index'))
                        script = home / 'script'
                        script.write_text(f':{3 if reverse else 5}\n:enter\n:maximize\n:4\n:status-update\n:quit\n')
                        env['TIG_SCRIPT'] = str(script)
                        code, timeout, transcript = upstream.terminal([str(binary), '-C', str(repo), 'status'], env, 15)
                        mode = git('ls-files', '--stage').split()[0].decode()
                        safe = path.read_bytes() == content and path.stat().st_mode & 0o777 == new_mode
                        refused = stale and binary.parent.name != 'src'
                        snapshot = home / 'stale-index'
                        unchanged_index = not stale or (snapshot.exists() and snapshot.read_bytes() == (repo / '.git/index').read_bytes())
                        passed = (code == (1 if refused else 0) and not timeout and mode == f'100{expected_mode:o}'
                                  and git('show', ':space name') == content and safe and unchanged_index
                                  and (not refused or 'expected 100' in transcript))
                        report['checks'].append(dict(binary=str(binary), binary_sha256=upstream.sha256(binary),
                            executable=executable, reverse=reverse, stale=stale, exit_code=code, timed_out=timeout,
                            index_mode=mode, index_preserved_after_stale=unchanged_index, worktree_preserved=safe,
                            passed=passed, transcript=transcript if not passed or refused else ''))
    args.output.write_text(json.dumps(report, indent=2) + '\n')
    assert all(check['passed'] for check in report['checks']), str(args.output)
    print('8 C/Rust mode-only parity checks and 4 Rust stale-index refusals passed')


if __name__ == '__main__':
    main()
