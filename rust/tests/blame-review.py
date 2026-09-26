#!/usr/bin/env python3
"""Paired C/Rust blame probes. Build both release binaries before running."""
import hashlib
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
    receipts = []
    with tempfile.TemporaryDirectory(prefix='tig-blame-review-') as temporary:
        cases = [('file', 'old'), ('deletion', '-- deleted text'), ('space name', 'a'), ('tab\tname', 'a')]
        for index, (filename, old) in enumerate(cases):
            repo = Path(temporary) / str(index)
            repo.mkdir()
            env = upstream.environment([])
            env.update(HOME=str(repo), GIT_CONFIG_GLOBAL='/dev/null', TIGRC_SYSTEM='',
                       TIGRC_USER=str(repo / 'tigrc'), TIG_SCRIPT=str(repo / 'steps'),
                       TERM='xterm-256color', LINES='30', COLUMNS='200')
            (repo / 'tigrc').write_text('set line-graphics = ascii\nset vertical-split = no\nset blame-view = line-number:yes,interval=1 text\n')

            def git(*args):
                return subprocess.check_output(['git', *args], cwd=repo, env=env).decode().strip()

            def check(label, binary, args, steps, expected):
                screen = repo / 'screen'
                screen.unlink(missing_ok=True)
                (repo / 'steps').write_text(steps + f'\n:save-display {screen}\n:quit\n')
                code, timeout, transcript = upstream.terminal(
                    [str(binary), '-C', str(repo), *args], env, 10)
                actual = screen.read_text() if screen.exists() else transcript
                passed = (code == 0 and not timeout and
                          any(row.startswith(f'[blame] {expected}:') and 'line 2 of 3' in row
                              for row in actual.splitlines()))
                receipts.append({'name': label, 'binary': str(binary.relative_to(ROOT)),
                                 'binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(),
                                 'expected_revision': expected, 'exit_code': code,
                                 'timed_out': timeout, 'passed': passed, 'screen': actual})
                print(f'{"PASS" if passed else "FAIL"}: {label}', flush=True)

            git('init', '-q')
            git('config', 'user.name', 'Blame Review')
            git('config', 'user.email', 'blame-review@example.invalid')
            (repo / filename).write_text(f'first\n{old}\nlast\n')
            git('add', '--', filename)
            git('-c', 'commit.gpgsign=false', 'commit', '-qm', 'old lines')
            before = git('rev-parse', 'HEAD')
            (repo / filename).write_text('first\nnew\nlast\n')
            git('-c', 'commit.gpgsign=false', 'commit', '-qam', 'replacement lines')
            after = git('rev-parse', 'HEAD')
            for mode, binary in [('c', ROOT / 'src/tig'), ('rust', ROOT / 'target/release/tig')]:
                # C cannot trace C-quoted tab paths; keep these as Rust regressions.
                if '\t' in filename and mode == 'c':
                    continue
                if filename == 'file':
                    check(f'{mode}: replacement Enter round trip', binary, ['blame', filename],
                          ':2\n:enter\n:view-blame', before)
                else:
                    check(f'{mode}: deleted line in {filename!r}', binary, ['show', 'HEAD', '--', filename],
                          f':/-{old}\n:view-blame', before)
                    check(f'{mode}: added line in {filename!r}', binary, ['show', 'HEAD', '--', filename],
                          ':/[+]new\n:view-blame', after)
            assert not git('status', '--porcelain', '--untracked-files=no')
    evidence = {'scope': 'Ten paired C/Rust probes and two Rust quoted-tab regressions; replacement Enter preserves C semantics. C quoted-tab tracing stays in diff and is not a parity pass.',
                'source_commit': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
                'checks': receipts}
    output = ROOT / 'migration/evidence/blame-review.json'
    output.write_text(json.dumps(evidence, indent=2) + '\n')
    assert all(item['passed'] for item in receipts), f'Failed probes: {output}'
    print(f'{len(receipts)} review probes passed: {output}')


if __name__ == '__main__':
    main()
