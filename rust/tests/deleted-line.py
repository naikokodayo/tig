#!/usr/bin/env python3
"""Real-index deleted-file partial unstage; retain C's newline-loss observations."""
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
              for name in ('rust/patch.rs', 'rust/main.rs', 'rust/git.rs', 'src/stage.c', 'rust/tests/deleted-line.py')}}
    for binary in (ROOT / 'src/tig', ROOT / 'target/release/tig'):
        for mode in (0o644, 0o755):
            for newline in (False, True):
                for selected, action in [('first', 'stage-update-line'), ('middle', 'stage-update-line'),
                                         ('last', 'stage-update-line'), ('middle', 'stage-update-part')]:
                    with tempfile.TemporaryDirectory(prefix='tig-deleted-') as temporary:
                        home = Path(temporary)
                        repo = home / 'repo'
                        repo.mkdir()
                        env = upstream.environment([binary.parent])
                        env.update(HOME=str(home), TIGRC_SYSTEM='', TIGRC_USER='/dev/null', TERM='xterm')
                        def git(*argv):
                            return subprocess.check_output(['git', *argv], cwd=repo, env=env, stderr=subprocess.STDOUT)
                        git('init', '-q')
                        git('config', 'user.name', 'Deleted Fixture')
                        git('config', 'user.email', 'deleted@example.invalid')
                        git('config', 'core.filemode', 'true')
                        path = repo / 'space name'
                        original = b'first\nmiddle\nlast' + (b'\n' if newline else b'')
                        path.write_bytes(original)
                        path.chmod(mode)
                        git('add', '--', path.name)
                        git('commit', '-qm', 'base')
                        path.unlink()
                        git('add', '-u')
                        script = home / 'script'
                        script.write_text(f':3\n:enter\n:maximize\n:/-{selected}\n:{action}\n:quit\n')
                        env['TIG_SCRIPT'] = str(script)
                        code, timeout, transcript = upstream.terminal([str(binary), '-C', str(repo), 'status'], env, 15)
                        blob = subprocess.run(['git', 'show', ':space name'], cwd=repo, env=env, capture_output=True)
                        entries = git('ls-files', '--stage').split()
                        actual_mode = entries[0].decode() if entries else None
                        expected = original if action == 'stage-update-part' else selected.encode() + (b'\n' if newline or selected != 'last' else b'')
                        known_loss = binary.parent.name == 'src' and not newline and selected != 'last' and action == 'stage-update-line'
                        safe = not path.exists() and blob.returncode == 0 and actual_mode == f'100{mode:o}'
                        observed_reference = expected.rstrip(b'\n') if known_loss else expected
                        report['checks'].append(dict(binary=str(binary), binary_sha256=upstream.sha256(binary),
                            mode=mode, newline=newline, selected=selected, action=action, exit_code=code, timed_out=timeout,
                            actual=blob.stdout.decode(), expected=expected.decode(), index_mode=actual_mode,
                            worktree_absent=not path.exists(), matches_content_oracle=blob.stdout == expected,
                            known_c_newline_loss=known_loss,
                            observed_as_expected=code == 0 and not timeout and safe and blob.stdout == observed_reference,
                            transcript=transcript if code or timeout else ''))
    args.output.write_text(json.dumps(report, indent=2) + '\n')
    assert all(c['observed_as_expected'] for c in report['checks']), str(args.output)
    print('Rust: 16/16 content checks; C: 12/16 match, 4 recorded newline-loss differences')


if __name__ == '__main__':
    main()
