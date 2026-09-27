#!/usr/bin/env python3
"""Real pipe + controlling PTY checks for `tig show --stdin`, paired with C."""
import argparse
import importlib.util
import json
import re
from pathlib import Path
import subprocess
import tempfile
import time

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location('upstream', ROOT / 'rust/tests/upstream-suite.py')
upstream = importlib.util.module_from_spec(spec)
spec.loader.exec_module(upstream)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--before", action="store_true")
    options = parser.parse_args()
    receipts = []
    with tempfile.TemporaryDirectory(prefix='tig-stdin-show-') as temporary:
        directory = Path(temporary)
        env = upstream.environment([])
        env.update(HOME=str(directory), GIT_CONFIG_GLOBAL='/dev/null', TIGRC_SYSTEM='',
                   TIGRC_USER='/dev/null', LINES='30', COLUMNS='80',
                   GIT_AUTHOR_DATE='2020-01-01T00:00:00+0000',
                   GIT_COMMITTER_DATE='2020-01-01T00:00:00+0000')

        def git(*args):
            return subprocess.check_output(['git', '-C', str(directory), *args], env=env).strip()

        git('init', '-q')
        git('config', 'user.name', 'Tester')
        git('config', 'user.email', 'test@example.com')
        (directory / 'sub').mkdir()
        commits = []
        for number, title in enumerate(['first', 'second', 'third']):
            (directory / 'sub/space name').write_text('line\n' * number + title + '\n')
            (directory / '--output=literal').write_text(title + '\n')
            git('add', '--all')
            git('commit', '-q', '-m', title)
            commits.append(git('rev-parse', 'HEAD'))
        git('notes', '--ref=review', 'add', '-m', 'REVIEW_NOTE', commits[1].decode())
        git('checkout', '-q', '--detach')
        (directory / 'sub/space name').write_text('white word\n')
        git('commit', '-qam', 'whitespace base')
        (directory / 'sub/space name').write_text('white  word\n')
        git('commit', '-qam', 'whitespace')
        whitespace = git('rev-parse', 'HEAD')
        git('checkout', '-q', 'master')
        blob = subprocess.check_output(['git', '-C', str(directory), 'hash-object', '-w', '--stdin'],
                                       input=b'commit ' + b'a' * 40 + b'\nvalid blob data\n', env=env).strip()
        bulk = commits[2]
        tree = git('rev-parse', 'HEAD^{tree}').decode()
        for number in range(120):
            bulk = git('commit-tree', tree, '-p', bulk.decode(), '-m', f'bulk {number}')
        cases = [
            ('many-commits', [], commits[2] + b'..' + bulk + b'\n', []),
            ('tagged-blob-text', [], blob + b'\n', []),
            ('blob-commit-text', [], blob + b'\n', []),
            ('ignore-space-all', [], whitespace + b'\n', []),
            ('ignore-space-some', [], whitespace + b'\n', []),
            ('ignore-space-at-eol', [], whitespace + b'\n', []),
            ('ignore-space-no', [], whitespace + b'\n', []),
            ('tagged-range', [], commits[0] + b'..' + commits[2] + b'\n', ['third', 'second']),
            ('mailmap-no', [], commits[1] + b'\n', ['second']),
            ('mailmap-yes', [], commits[1] + b'\n', ['second']),
            ('notes', [], commits[1] + b'\n', ['second']),
            ('one', [], commits[1] + b'\n', ['second']),
            ('navigate-tree', [], commits[1] + b'\n', ['second']),
            ('navigate-second-tree', [], commits[0] + b'..' + commits[2] + b'\n', ['third', 'second']),
            ('navigate-blame', [], commits[1] + b'\n', ['second']),
            ('empty', [], b'', ['third']),
            ('duplicate', [], (commits[1] + b'\n') * 2, ['second']),
            ('no-newline', [], commits[0], ['first']),
            ('navigate-second-stat', [], commits[0] + b'..' + commits[2] + b'\n', ['third', 'second']),
            ('context', ['-U0'], commits[1] + b'\n', ['second']),
            ('range', [], commits[0] + b'..' + commits[2] + b'\n', ['third', 'second']),
            ('exclude', [], commits[2] + b'\n^' + commits[0] + b'\n', ['third', 'second']),
            ('large', [], (commits[1] + b'\n') * 10000, ['second']),
            ('cli-path', ['--', 'sub/space name'], commits[1] + b'\n', ['second']),
            ('stdin-path', [], commits[1] + b'\n--\nsub/space name\n', ['second']),
            ('literal-option-path', [], commits[1] + b'\n--\n--output=literal\n', ['second']),
            ('subdir-cli', ['--', 'space name'], commits[1] + b'\n', ['second']),
            ('subdir-stdin', [], commits[1] + b'\n--\nspace name\n', ['second']),
            ('two-paths', ['--', 'sub/space name', '--output=literal'], commits[1] + b'\n', ['second']),
            ('combined-revisions', ['HEAD~1'], commits[0] + b'\n', ['first', 'second']),
            ('invalid', [], b'missing-revision\n', None),
            ('format-injection', [], b'--format=evil\nHEAD\n', None),
            ('output-injection', [], b'--output=stolen\nHEAD\n', None),
            ('nul-injection', [], b'HEAD\0ignored\n', None),
        ]
        for name, args, raw, titles in cases:
            input_file = directory / 'input'
            input_file.write_bytes(raw)
            config = directory / 'tigrc'
            config.write_text('set show-notes = refs/notes/review\n' if name == 'notes' else
                              'set mailmap = ' + name.removeprefix('mailmap-') + '\n'
                              if name.startswith('mailmap-') else
                              'set ignore-space = ' + name.removeprefix('ignore-space-') + '\n'
                              if name.startswith('ignore-space-') else '')
            if name.startswith('mailmap-'):
                (directory / '.mailmap').write_text('Mapped <mapped@example.com> Tester <test@example.com>\n')
            else:
                (directory / '.mailmap').unlink(missing_ok=True)
            if name in ('tagged-range', 'tagged-blob-text'):
                git('tag', '-a', 'v1', '-m', 'version', commits[0].decode())
            paired = {}
            for mode, binary in [('c', ROOT / 'src/tig'), ('rust', ROOT / 'target/release/tig')]:
                # C's NUL handling is intentionally not the safety oracle.
                if name == 'nul-injection' and mode == 'c':
                    continue
                view = directory / f'{mode}.view'
                refreshed = directory / f'{mode}.refresh'
                screen = directory / f'{mode}.screen'
                navigation = directory / f'{mode}.navigation'
                for path in [view, refreshed, screen, navigation]:
                    path.unlink(missing_ok=True)
                steps = directory / 'steps'
                navigate = (f':/[+]second\n:view-blame\n:save-display {navigation}\n:view-close\n:move-first-line\n'
                            if name == 'navigate-blame' else
                            f':/....second\n:move-down\n:move-down\n:enter\n:save-view {navigation}\n:move-first-line\n'
                            if name == 'navigate-second-stat' else
                            (f':/{commits[1].decode()}\n' if name == 'navigate-second-tree' else '') +
                            f':view-tree\n:save-view {navigation}\n:view-close\n:move-first-line\n'
                            if name in ('navigate-tree', 'navigate-second-tree') else '')
                steps.write_text(f':save-view {view}\n:save-display {screen}\n' + navigate +
                                 f':refresh\n:save-view {refreshed}\n:quit\n')
                trace = directory / f'{mode}.trace'
                trace.unlink(missing_ok=True)
                started = time.monotonic()
                code, timeout, transcript = upstream.terminal(
                    ['sh', '-c', 'input=$1; binary=$2; repo=$3; shift 3; cat "$input" | "$binary" -C "$repo" show --stdin "$@"',
                     'stdin-show', str(input_file), str(binary),
                     str(directory / 'sub' if name.startswith('subdir-') else directory), *args],
                    {**env, 'TIG_SCRIPT': str(steps), 'TIGRC_USER': str(config), 'GIT_TRACE': str(trace)}, 20)
                elapsed = time.monotonic() - started
                assert not timeout, (name, mode, transcript)
                if options.before and mode == 'rust':
                    assert code != 0 and 'Forwarding revision input' in transcript, (name, transcript)
                    receipts.append({'case': name, 'mode': mode, 'expected_rejection': True})
                    continue
                if titles is None:
                    assert code != 0 or not view.exists() or 'fatal:' in transcript, (name, mode, transcript)
                    if mode == 'rust':
                        assert code != 0, (name, transcript)
                else:
                    assert code == 0 and view.exists(), (name, mode, code, transcript)
                    shown = view.read_text(errors='replace')
                    found = re.findall(r'text=\[    (first|second|third)\]', shown)
                    assert found == titles, (name, mode, shown)
                    if name == 'many-commits':
                        assert len(re.findall(r'cells=1 text=\[commit [0-9a-f]+\]', shown)) == 120, (mode, shown)
                        calls = trace.read_text().count('built-in: git')
                        if mode == 'rust':
                            assert calls < 40, (calls, trace.read_text())
                        receipts.append({'case': name, 'mode': mode, 'git_calls': calls, 'seconds': elapsed})
                    if name in ('blob-commit-text', 'tagged-blob-text'):
                        assert 'valid blob data' in shown, (mode, shown)
                    if name.startswith('ignore-space-'):
                        assert ('type=diff-chunk' in shown) == (name in ('ignore-space-no', 'ignore-space-at-eol')), (mode, shown)
                    if name.startswith('mailmap-'):
                        assert ('Mapped <mapped@example.com>' in shown) == (name == 'mailmap-yes'), (mode, shown)
                    if name == 'notes':
                        assert 'REVIEW_NOTE' in shown, (mode, shown)
                    paired[mode] = shown
                    paired[mode + '-refresh'] = refreshed.read_text()
                    if name == 'navigate-blame':
                        navigated = navigation.read_text()
                        assert '[blame]' in navigated and 'second' in navigated, (mode, navigated)
                        paired[mode + '-navigation'] = navigated
                if name in ('navigate-tree', 'navigate-second-tree') and titles is not None:
                    navigated = navigation.read_text()
                    assert 'View: tree' in navigated, (mode, navigated)
                    assert git('rev-parse', commits[1].decode() + ':sub').decode() in navigated, (mode, navigated)
                    paired[mode + '-navigation'] = navigated
                if name == 'navigate-second-stat' and titles is not None:
                    navigated = navigation.read_text()
                    assert 'type=diff-header selected=1' in navigated, (mode, navigated)
                    paired[mode + '-navigation'] = navigated
                assert not (directory / 'stolen').exists(), (name, mode, 'wrote injected output')
                receipts.append({'case': name, 'mode': mode, 'exit': code, 'pass': True})
            if titles is not None and not options.before:
                if name == 'tagged-blob-text':
                    # C describes invalid commit-shaped blob prose with an unrelated tag.
                    c_text = re.findall(r'cells=1 text=\[(.*)\]', paired['c'])
                    rust_text = re.findall(r'cells=1 text=\[(.*)\]', paired['rust'])
                    assert len(c_text) == 3 and c_text[1].startswith('Refs: v1'), paired
                    assert [c_text[0], c_text[2]] == rust_text, paired
                    receipts.append({'case': name,
                                     'intentional_difference': 'Do not describe invalid commit-shaped blob prose with an unrelated tag',
                                     'c_export': paired['c'], 'rust_export': paired['rust'],
                                     'c_refresh': paired['c-refresh'], 'rust_refresh': paired['rust-refresh']})
                else:
                    assert paired['c'] == paired['rust'], (name, paired)
                if name == 'tagged-blob-text':
                    description = git('describe', '--tags', 'HEAD').decode()
                    expected = paired['c-refresh'].replace('text=[Refs: [master]]',
                                                           f'text=[Refs: [master], {description}]', 1)
                    assert expected == paired['rust-refresh'], (name, 'refresh', paired)
                else:
                    assert paired['c-refresh'] == paired['rust-refresh'], (name, 'refresh', paired)
                if name in ('navigate-blame', 'navigate-tree', 'navigate-second-tree'):
                    assert paired['c-navigation'] == paired['rust-navigation'], paired
            if name in ('tagged-range', 'tagged-blob-text'):
                git('tag', '-d', 'v1')
            if name == 'navigate-second-stat' and not options.before:
                # C jumps to the first file with this name across all commits.
                # Rust deliberately keeps the target in the selected commit.
                for mode, expected in [('c', commits[2]), ('rust', commits[1])]:
                    data = paired[mode + '-navigation']
                    selected = data.index('type=diff-header selected=1')
                    header = re.findall(r'text=\[commit ([0-9a-f]+)\]', data[:selected])[-1]
                    assert header == expected.decode(), (mode, data)
                receipts.append({'case': name,
                                 'intentional_difference': 'C jumps to first commit; Rust stays in selected second commit',
                                 'c_export': paired['c-navigation'],
                                 'rust_export': paired['rust-navigation']})
    print(json.dumps(receipts, indent=2))


if __name__ == '__main__':
    main()
