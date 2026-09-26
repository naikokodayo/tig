#!/usr/bin/env python3
"""Paired public-CLI checks for revision stdin; run after building both binaries."""
import importlib.util
import json
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location('upstream', ROOT / 'rust/tests/upstream-suite.py')
upstream = importlib.util.module_from_spec(spec)
spec.loader.exec_module(upstream)

with tempfile.TemporaryDirectory(prefix='tig-stdin-') as temporary:
    directory = Path(temporary)
    env = upstream.environment([])
    env.update(HOME=str(directory), GIT_CONFIG_GLOBAL='/dev/null', TIGRC_SYSTEM='',
               TIGRC_USER='/dev/null', LINES='10', COLUMNS='80')
    def git(*args):
        return subprocess.check_output(['git', '-C', str(directory), *args], env=env).decode().strip()
    git('init', '-q')
    git('config', 'user.name', 'Tester')
    git('config', 'user.email', 'test@example.com')
    commits = []
    for title in ['first', 'second', 'third']:
        git('commit', '-q', '--allow-empty', '-m', title)
        commits.append(git('rev-parse', 'HEAD'))
    cases = [
        ('one', '--no-walk', commits[1] + '\n', ['second']),
        ('empty', '--no-walk', '', ['third']),
        ('duplicate', '--no-walk', (commits[1] + '\n') * 2, ['second']),
        ('no-newline', '--no-walk', commits[0], ['first']),
        ('range', '--no-walk', commits[0] + '..' + commits[2] + '\n', ['third', 'second']),
        ('exclude', '--no-walk', commits[2] + '\n^' + commits[0] + '\n', ['third', 'second']),
        ('large', '--no-walk', (commits[1] + '\n') * 10000, ['second']),
        ('walk', '--topo-order', commits[2] + '\n', ['third', 'second', 'first']),
        ('non-utf8-ref', '--no-walk', b'byte-\xff\n', ['second']),
        ('invalid', '--no-walk', 'missing-revision\n', None),
        ('format-injection', '--no-walk', '--format=evil\nHEAD\n', None),
    ]
    receipts = []
    for name, option, raw, titles in cases:
        input_file = directory / 'input'
        if name == 'non-utf8-ref':
            # Packed refs also work on filesystems that reject non-UTF-8 filenames.
            (directory / '.git/packed-refs').write_bytes(
                commits[1].encode() + b' refs/heads/byte-\xff\n')
        input_file.write_bytes(raw if isinstance(raw, bytes) else raw.encode())
        config = directory / 'tigrc'
        # Isolate ref lookup from existing C/Rust invalid-byte decoration rendering.
        config.write_text('set main-view-commit-title = yes,refs=no,graph=no\n'
                          if name == 'non-utf8-ref' else '')
        screens = {}
        for mode, binary in [('c', ROOT / 'src/tig'), ('rust', ROOT / 'target/release/tig')]:
            screen = directory / f'{mode}.screen'
            refreshed = directory / f'{mode}.refresh'
            screen.unlink(missing_ok=True)
            refreshed.unlink(missing_ok=True)
            steps = directory / 'steps'
            steps.write_text(f':save-display {screen}\n:refresh\n:save-display {refreshed}\n:quit\n')
            code, timeout, transcript = upstream.terminal(
                ['sh', '-c', 'exec "$1" -C "$2" "$3" --stdin < "$4"',
                 'stdin-test', str(binary), str(directory), option, str(input_file)],
                {**env, 'TIG_SCRIPT': str(steps), 'TIGRC_USER': str(config)}, 15)
            assert not timeout, (name, mode, transcript)
            if titles is None:
                assert code != 0 or not screen.exists(), (name, mode, transcript)
                if mode == 'rust':
                    assert code != 0, (name, transcript)
            else:
                assert code == 0 and screen.exists(), (name, mode, code, transcript)
                shown = screen.read_text(errors='replace')
                found = [title for line in shown.splitlines() for title in ['first', 'second', 'third']
                         if line.rstrip().endswith(title)]
                assert found == titles and '[main]' in shown, (name, mode, shown)
                assert refreshed.read_text(errors='replace') == shown, (name, mode, 'refresh changed input')
                # Match the original harness's trailing-space normalization.
                screens[mode] = '\n'.join(line.rstrip() for line in shown.splitlines())
            receipts.append({'case': name, 'mode': mode, 'exit': code, 'pass': True})
        if titles is not None:
            assert screens['c'] == screens['rust'], (name, screens)
        if name == 'non-utf8-ref':
            (directory / '.git/packed-refs').unlink()
    print(json.dumps(receipts, indent=2))
