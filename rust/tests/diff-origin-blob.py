#!/usr/bin/env python3
"""Real Git/PTY origin-blob regression; build C and Rust release binaries first."""
import argparse
import importlib.util
import json
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location('upstream', ROOT / 'rust/tests/upstream-suite.py')
harness = importlib.util.module_from_spec(spec)
spec.loader.exec_module(harness)
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--output', type=Path, required=True)
args = parser.parse_args()
args.output.unlink(missing_ok=True)
report = {'source_sha256': {name: harness.sha256(ROOT / name) for name in
          ('rust/main.rs', 'rust/git.rs', 'src/diff.c', 'src/stage.c', 'rust/tests/diff-origin-blob.py')},
          'checks': []}
with tempfile.TemporaryDirectory(prefix='tig-origin-blob-') as temporary:
    home = Path(temporary)
    repo = home / 'repo'
    repo.mkdir()
    env = harness.environment([])
    env.update(HOME=str(home), GIT_CONFIG_GLOBAL='/dev/null', TIGRC_SYSTEM='',
               TIGRC_USER=str(home / 'tigrc'), TIG_SCRIPT=str(home / 'steps'),
               TERM='xterm', LINES='30', COLUMNS='160')
    (home / 'tigrc').write_text('set line-graphics = ascii\nset vertical-split = no\n')

    def git(*argv):
        return subprocess.check_output(['git', *argv], cwd=repo, env=env).decode().strip()

    git('init', '-q')
    git('config', 'user.name', 'Origin Fixture')
    git('config', 'user.email', 'origin@example.invalid')
    old = 'first\nold marker\nthird\nfourth\nfifth\n'
    new = old.replace('old marker', 'new marker')
    (repo / 'old name').write_text(old)
    git('add', '.')
    git('-c', 'commit.gpgsign=false', 'commit', '-qm', 'base')
    base = git('rev-parse', 'HEAD')
    git('mv', 'old name', 'new name')
    (repo / 'new name').write_text(new)
    git('-c', 'commit.gpgsign=false', 'commit', '-qam', 'rename and replace')
    head = git('rev-parse', 'HEAD')
    (repo / 'new name').write_text(new.replace('new marker', 'staged marker') + 'appended marker\n')
    git('add', '.')
    index = git('write-tree')
    worktree = (repo / 'new name').read_bytes()
    cases = [
        ('deleted origin', ['show', head], ':/-old marker', old, 2),
        ('added origin', ['show', head], ':/[+]new marker', new, 2),
        ('context historical rename', ['show', head], ':/third', old, 3),
        ('stat current blob', ['show', head], ':/=>', new, 1),
        ('root added line', ['show', base], ':/[+]old marker', old, 2),
        ('staged deletion', ['status'], ':3\n:enter\n:maximize\n:/-new marker', new, 2),
        ('staged EOF addition', ['status'], ':3\n:enter\n:maximize\n:/[+]appended marker', new, 5),
        ('staged addition', ['status'], ':3\n:enter\n:maximize\n:/[+]staged marker', new, 3),
    ]
    for binary in (ROOT / 'src/tig', ROOT / 'target/release/tig'):
        for label, argv, steps, expected, number in cases:
            screen = home / 'screen'
            screen.unlink(missing_ok=True)
            (home / 'steps').write_text(steps + f'\n:view-blob\n:save-display {screen}\n:quit\n')
            code, timeout, transcript = harness.terminal([str(binary), '-C', str(repo), *argv], env, 15)
            actual = screen.read_text() if screen.exists() else transcript
            content = all(line in actual for line in expected.splitlines())
            title = any(row.startswith('[blob]') and f'line {number} of 5' in row
                        for row in actual.splitlines())
            safe = git('write-tree') == index and (repo / 'new name').read_bytes() == worktree
            passed = code == 0 and not timeout and content and title and safe
            report['checks'].append(dict(name=label, binary=str(binary.relative_to(ROOT)),
                binary_sha256=harness.sha256(binary), passed=passed, exit_code=code,
                timeout=timeout, unchanged=safe, screen=actual))
            print(f'{binary.parent.name}: {label}: {passed}', flush=True)
args.output.parent.mkdir(parents=True, exist_ok=True)
args.output.write_text(json.dumps(report, indent=2) + '\n')
assert all(check['passed'] for check in report['checks']), args.output
