#!/usr/bin/env python3
"""Real-PTY regressions for wrapped title counts and source-line blame tracing.

Build C and Rust release binaries first. Prints evidence to stdout; the caller
includes it in the slice's single final receipt.
"""
import hashlib
import importlib.util
import json
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location('upstream', ROOT / 'rust/tests/upstream-suite.py')
harness = importlib.util.module_from_spec(spec)
spec.loader.exec_module(harness)


def main():
    checks = []
    with tempfile.TemporaryDirectory(prefix='tig-wrap-rendering-') as temporary:
        directory = Path(temporary)
        repo = directory / "repo"
        repo.mkdir()
        env = harness.environment([])
        env.update(HOME=str(directory), GIT_CONFIG_GLOBAL='/dev/null', TIGRC_SYSTEM='',
                   TIGRC_USER=str(directory / 'tigrc'), TIG_SCRIPT=str(directory / 'steps'),
                   TERM='xterm-256color', LINES='30', LC_ALL='C')
        config = 'set line-graphics = ascii\nset blame-view = line-number:yes,interval=1 text\n'

        def git(*args):
            return subprocess.check_output(['git', '-C', str(repo), *args], env=env).decode().strip()

        def run(mode, args, steps, width=80, wrapped=True):
            binary = ROOT / ('src/tig' if mode == 'c' else 'target/release/tig')
            target = directory / 'screen'
            target.unlink(missing_ok=True)
            (directory / 'tigrc').write_text(config + f'set wrap-lines = {str(wrapped).lower()}\n')
            (directory / 'steps').write_text(steps + f'\n:save-display {target}\n:quit\n')
            code, timeout, transcript = harness.terminal(
                [str(binary), '-C', str(repo), *args], {**env, 'COLUMNS': str(width)}, 15)
            screen = target.read_text() if target.exists() else ''
            result = {'mode': mode, 'exit_code': code, 'timed_out': timeout,
                      'screen': screen, 'transcript': transcript}
            return result

        git('init', '-q')
        git('config', 'user.name', 'Wrap Test')
        git('config', 'user.email', 'wrap@example.invalid')
        for label in ('OLD', 'NEW'):
            (repo / 'f').write_text('first\n' + label + 'x' * 190 + '\nlast\n')
            git('add', 'f')
            git('-c', 'commit.gpgsign=false', 'commit', '-qm', label)
            if label == 'OLD':
                before = git('rev-parse', 'HEAD')
        after = git('rev-parse', 'HEAD')

        for mode in ('c', 'rust'):
            result = run(mode, [], ':view-tree\n:/f\n:view-blob\n:2\n:move-down')
            result.update(name='blob continuation title', passed='[blob] f - line 2 of 3' in result['screen'])
            checks.append(result)
            result = run(mode, ['show', 'HEAD'], ':/[+]NEW\n:move-down')
            title = next((line for line in result['screen'].splitlines() if line.startswith('[diff]')), '')
            result.update(name='diff continuation title', title=title)
            checks.append(result)

        titles = [item for item in checks if item['name'] == 'diff continuation title']
        for item in titles:
            item['passed'] = bool(titles[0]['title']) and item['title'] == titles[0]['title']
        for prefix, revision in (('-OLD', before), ('[+]NEW', after)):
            for mode in ('c', 'rust'):
                # C's trace loop counts wrapped patch fragments too. Use its
                # unwrapped source-line result as the reference for Rust's
                # continuation selection; do not certify that C bug as parity.
                steps = f':/{prefix}\n' + (':move-down\n' if mode == 'rust' else '') + ':view-blame'
                result = run(mode, ['show', 'HEAD'], steps, width=160, wrapped=mode == 'rust')
                result.update(name=f'trace {prefix} source line', expected_revision=revision,
                              passed=any(line.startswith(f'[blame] {revision}:f') and 'line 2 of 3' in line
                                         for line in result['screen'].splitlines()))
                checks.append(result)
        assert not git('status', '--porcelain', '--untracked-files=no'), 'navigation changed tracked files'
    for check in checks:
        check['passed'] &= check['exit_code'] == 0 and not check['timed_out']
    evidence = {'source_commit': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
                'binary_sha256': {str(p.relative_to(ROOT)): hashlib.sha256(p.read_bytes()).hexdigest()
                                  for p in (ROOT / 'src/tig', ROOT / 'target/release/tig')},
                'checks': checks}
    print(json.dumps(evidence, indent=2))
    assert all(check['passed'] for check in checks), 'Wrapped terminal regression failed'


if __name__ == '__main__':
    main()
