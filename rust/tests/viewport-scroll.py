#!/usr/bin/env python3
"""Compare page scrolling through C/Rust Git + PTY views; stdlib only."""
import argparse
import importlib.util
from pathlib import Path
import re
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location('upstream', ROOT / 'rust/tests/upstream-suite.py')
h = importlib.util.module_from_spec(spec)
spec.loader.exec_module(h)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--rust-binary', type=Path, default=ROOT / 'target/release/tig')
    args = parser.parse_args()
    with tempfile.TemporaryDirectory(prefix='tig-viewport-scroll-') as temporary:
        directory = Path(temporary)
        repo = directory / 'repo'
        repo.mkdir()
        env = h.environment([])
        env.update(HOME=str(directory), GIT_CONFIG_GLOBAL='/dev/null', TIGRC_SYSTEM='',
                   TIGRC_USER=str(directory / 'tigrc'), TIG_SCRIPT=str(directory / 'steps'),
                   LC_ALL='C', TERM='xterm-256color', GIT_AUTHOR_NAME='Test',
                   GIT_AUTHOR_EMAIL='test@example.invalid', GIT_COMMITTER_NAME='Test',
                   GIT_COMMITTER_EMAIL='test@example.invalid')

        def git(*argv):
            return subprocess.check_output(['git', '-C', str(repo), *argv], env=env).decode().strip()

        git('init', '-q')
        source = repo / 'lines.txt'
        source.write_text(''.join(f'row {i:03d} ' + 'content ' * 12 + '\n' for i in range(137)))
        git('add', '.')
        git('-c', 'commit.gpgsign=false', 'commit', '-qm', 'long blob')
        pager_input = directory / 'pager-input'
        pager_input.write_text('commit ' + git('rev-parse', 'HEAD') + '\n' + source.read_text())
        # Odd heights exercise integer half-pages; split widths must not become page heights.
        cases = [
            ('pager', 81, 19, '', ''),
            ('blob', 81, 19, '', ':view-tree\n:move-last-line\n:enter\n:maximize\n'),
            ('horizontal-child', 81, 20, 'set vertical-split = no\nset split-view-height = 60%\n',
             ':view-tree\n:move-last-line\n:enter\n'),
            ('vertical-child', 83, 19, 'set vertical-split = yes\nset split-view-width = 37\n',
             ':view-tree\n:move-last-line\n:enter\n'),
            ('pager-parent-horizontal', 81, 20, 'set vertical-split = no\nset split-view-height = 60%\n',
             ':enter\n:view-next\n'),
            ('pager-parent-vertical', 83, 19, 'set vertical-split = yes\nset split-view-width = 37\n',
             ':enter\n:view-next\n'),
        ]
        total = 0
        for name, width, height, config, opening in cases:
            (directory / 'tigrc').write_text(config)
            captures = {}
            for mode, binary in [('c', ROOT / 'src/tig'), ('rust', args.rust_binary.resolve())]:
                # A nonzero cursor row distinguishes viewport scrolling from move-page.
                actions = ['', ':move-down\n' * 3, ':scroll-page-up', ':scroll-half-page-up',
                           ':scroll-page-down', ':scroll-half-page-down', ':scroll-half-page-up',
                           ':scroll-page-up', ':move-last-line', ':scroll-page-down',
                           ':scroll-half-page-down', ':scroll-page-up', ':scroll-half-page-up',
                           ':scroll-page-down', ':scroll-half-page-down', ':move-first-line',
                           ':scroll-page-up', ':scroll-half-page-up',
                           ':scroll-line-down', ':scroll-page-up', ':scroll-half-page-down',
                           ':scroll-page-up', ':move-last-line', ':scroll-line-up',
                           ':scroll-half-page-down', ':scroll-line-up', ':scroll-page-down']
                if opening:
                    actions.append(':view-next\n:view-close' if name.startswith('pager-parent') else ':view-close')
                paths = [directory / f'{name}-{mode}-{i}.view' for i in range(len(actions))]
                (directory / 'steps').write_text(opening + ''.join(
                    f'{action}\n:save-view {path}\n' for action, path in zip(actions, paths)) + ':quit\n')
                command = [str(binary), '-C', str(repo)]
                if name.startswith('pager'):
                    command = ['/bin/sh', '-c', 'exec "$1" -C "$2" < "$3"',
                               'sh', str(binary), str(repo), str(pager_input)]
                code, timeout, transcript = h.terminal(
                    command, {**env, 'COLUMNS': str(width), 'LINES': str(height)}, 20)
                assert code == 0 and not timeout, (name, mode, code, transcript)
                captures[mode] = []
                for path in paths:
                    data = path.read_text()
                    view = re.search(r'^View: (.+)$', data, re.M).group(1)
                    dimensions = tuple(map(int, re.search(r'Dimensions: height=(\d+) width=(\d+)', data).groups()))
                    position = tuple(map(int, re.search(r'Position: offset=(\d+) column=(\d+) lineno=(\d+)', data).groups()))
                    captures[mode].append((view, dimensions, position))
            assert captures['c'] == captures['rust'], (name, captures)
            initial = captures['c'][1]
            page = initial[1][0]
            assert initial[0] == ('pager' if name.startswith('pager') else 'blob'), initial
            assert captures['c'][4][2] == (page, 0, page + 3), captures['c']
            assert captures['c'][5][2] == (page + page // 2, 0, page + page // 2 + 3), captures['c']
            assert captures['c'][8] == captures['c'][9] == captures['c'][10], captures['c']
            assert captures['c'][15] == captures['c'][16] == captures['c'][17], captures['c']
            assert captures['c'][19] == captures['c'][21] == captures['c'][15], captures['c']
            assert captures['c'][24] == captures['c'][26] == captures['c'][8], captures['c']
            total += len(actions)
            print(f'PASS: {name}: {len(actions)} paired viewport/selection/dimension/return states', flush=True)
        assert not git('status', '--porcelain'), 'scrolling modified the Git fixture'
        print(f'PASS: {total} paired states; clean Git fixture')


if __name__ == '__main__':
    main()
