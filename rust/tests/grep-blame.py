#!/usr/bin/env python3
"""Real Git/PTY grep -> blame line positioning, against C and Rust."""
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
parser.add_argument('--rust-binary', type=Path, default=ROOT / 'target/release/tig')
args = parser.parse_args()
checks = []
with tempfile.TemporaryDirectory(prefix='tig-grep-blame-') as temporary:
    repo = Path(temporary)
    env = harness.environment([])
    env.update(GIT_CONFIG_GLOBAL='/dev/null', TIGRC_SYSTEM='', LC_ALL='C',
               TIGRC_USER=str(repo / 'tigrc'), TIG_SCRIPT=str(repo / 'steps'),
               COLUMNS='100', LINES='20')
    def git(*arguments):
        return subprocess.check_output(['git', '-C', str(repo), *arguments], env=env)
    git('init', '-q')
    git('config', 'user.name', 'Test')
    git('config', 'user.email', 'test@example.invalid')
    path = repo / 'file name.txt'
    lines = [f'line {i}\n' for i in range(1, 61)]
    lines[42] = 'NEEDLE committed\n'
    path.write_text(''.join(lines))
    git('add', '--', path.name)
    git('-c', 'commit.gpgsign=false', 'commit', '-qm', 'base')
    revision = git('rev-parse', 'HEAD').decode().strip()
    path.write_text('prefix\n' + ''.join(lines))
    git('-c', 'commit.gpgsign=false', 'commit', '-qam', 'shift committed line')
    path.write_text('worktree prefix\n' + path.read_text())
    before = git('status', '--porcelain', '--untracked-files=no')
    (repo / 'tigrc').write_text('set vertical-split = no\nset line-graphics = ascii\n'
                              'set blame-view = line-number:yes,interval=1 text\n')
    for label, revision_args, row, expected, context in [
        ('historical match', [revision], 2, 43, []),
        ('HEAD match', ['HEAD'], 2, 44, []),
        ('worktree match', [], 2, 45, []),
        ('file header', [revision], 1, 1, []),
        ('context row', [revision], 2, 42, ['-C1']),
    ]:
        for mode, binary in [('c', ROOT / 'src/tig'), ('rust', args.rust_binary.resolve())]:
            opened, refreshed, back = [repo / name for name in ('opened', 'refreshed', 'back')]
            for screen in (opened, refreshed, back):
                screen.unlink(missing_ok=True)
            (repo / 'steps').write_text(
                f':{row}\n:view-blame\n:save-display {opened}\n'
                f':refresh\n:save-display {refreshed}\n:view-close\n:save-display {back}\n:quit\n')
            code, timeout, transcript = harness.terminal(
                [str(binary), '-C', str(repo), 'grep', *context, 'NEEDLE', *revision_args,
                 '--', path.name], env, 10)
            assert code == 0 and not timeout, (label, mode, transcript)
            for screen in (opened, refreshed):
                text = screen.read_text()
                assert '[blame]' in text and f'line {expected} of ' in text, (label, mode, text)
                if expected > 1:
                    content = 'line 42' if context else 'NEEDLE committed'
                    assert f'{expected}| {content}' in text, (label, mode, text)
            assert '[grep]' in back.read_text() and f'line {row} of ' in back.read_text(), (label, mode, back.read_text())
            checks.append({'case': label, 'mode': mode, 'line': expected, 'pass': True})
    assert git('status', '--porcelain', '--untracked-files=no') == before, 'navigation modified tracked files'
print(json.dumps({'passed': len(checks), 'checks': checks}, indent=2))
