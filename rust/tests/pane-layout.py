#!/usr/bin/env python3
"""Compare pane redraw, scrolling, and describe headers in real C/Rust PTYs.

Build both release binaries first. Uses only stdlib and the original Git fixture;
prints checks for inclusion in the slice's single final receipt.
"""
import difflib
import importlib.util
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location('upstream', ROOT / 'rust/tests/upstream-suite.py')
upstream = importlib.util.module_from_spec(spec)
spec.loader.exec_module(upstream)


def main():
    with tempfile.TemporaryDirectory(prefix='tig-pane-layout-') as temporary:
        directory = Path(temporary)
        repo = directory / 'repo'
        repo.mkdir()
        env = upstream.environment([])
        env.update(HOME=str(directory), GIT_CONFIG_GLOBAL='/dev/null', TIGRC_SYSTEM='',
                   TIGRC_USER=str(directory / 'tigrc'), TIG_SCRIPT=str(directory / 'steps'),
                   TERM='xterm-256color', LC_ALL='en_US.UTF-8', GIT_AUTHOR_NAME='Test',
                   GIT_AUTHOR_EMAIL='test@example.invalid', GIT_COMMITTER_NAME='Test',
                   GIT_COMMITTER_EMAIL='test@example.invalid')

        def git(*args):
            return subprocess.check_output(['git', '-C', str(repo), *args], env=env).decode().strip()

        git('init', '-q')
        subprocess.run(['tar', 'xzf', str(ROOT / 'test/files/repo-one.tgz'), '-C', str(repo)], check=True)
        git('reset', '--hard', '-q')
        cases = [
            ('odd width and child refresh', 81, '', ':enter\n:refresh\n:set line-graphics = utf-8'),
            ('parent refresh', 80, '', ':enter\n:view-next\n:refresh'),
            ('selected long author', 80, '', ':move-down\n:enter'),
            ('minimum child width', 80, '', ':enter\n:set split-view-width = 0%'),
            ('minimum parent width', 80, '', ':enter\n:set split-view-width = 99%'),
            ('absolute child width', 81, '', ':enter\n:set split-view-width = 37'),
            ('scroll narrow parent', 80, '', ':enter\n:set split-view-width = 75%\n:view-next\n:scroll-right'),
            ('search offscreen title', 80, '', ':enter\n:set split-view-width = 75%\n:view-next\n:/Commit 9 D'),
            ('unicode delimiter', 80, 'set truncation-delimiter = "⋯"\n',
             ':enter\n:set main-view-line-number-display = yes'),
            ('scroll first column', 80, '', ':enter\n:view-next\n:scroll-right\n:scroll-first-col'),
            ('line scroll exposes rows', 80, '', ':enter\n:view-next\n:scroll-line-down'),
            ('relative date in narrow pane', 80, '', ':enter\n:set split-view-width = 90%\n:set main-view-date-display = relative'),
            ('narrow reflog author', 16, 'set reflog-view = author:full,width=24 commit-title\n', ':view-reflog'),
            ('restore parent width', 80, '', ':enter\n:set split-view-width = 75%\n:view-close'),
            ('lightweight describe only', 80, '', ':move-down\n:enter\n:maximize'),
        ]
        failures = []
        for annotated in (False, True):
            if annotated:
                git('-c', 'tag.gpgsign=false', 'tag', '-a', 'annotated', '-m', 'annotated', 'HEAD~3')
                cases = [('annotated describe only', 80, '', ':move-down\n:enter\n:maximize'),
                         ('preserve Unicode whitespace', 80, '', ':set main-view = commit-title:yes,graph=no,refs=no')]
            for name, width, extra, steps in cases:
                if name == 'preserve Unicode whitespace':
                    git('-c', 'commit.gpgsign=false', 'commit', '--allow-empty', '-qm', 'Subject ending in NBSP\u00a0')
                screens = []
                for mode, binary in [('c', ROOT / 'src/tig'), ('rust', ROOT / 'target/release/tig')]:
                    target = directory / 'screen'
                    target.unlink(missing_ok=True)
                    (directory / 'tigrc').write_text('set vertical-split = yes\n' + extra)
                    (directory / 'steps').write_text(steps + f'\n:save-display {target}\n:quit\n')
                    code, timeout, transcript = upstream.terminal(
                        [str(binary), '-C', str(repo)], {**env, 'COLUMNS': str(width), 'LINES': '18'}, 20)
                    assert code == 0 and not timeout and target.exists(), (name, mode, code, transcript)
                    screens.append(target.read_text())
                if screens[0] != screens[1]:
                    failures.append(name)
                    print(f'FAIL: {name}\n' + ''.join(difflib.unified_diff(
                        screens[0].splitlines(True), screens[1].splitlines(True), fromfile='c', tofile='rust')), flush=True)
                else:
                    print(f'PASS: {name}', flush=True)
        assert not git('status', '--porcelain'), 'pane operations changed the fixture'
        assert not failures, failures


if __name__ == '__main__':
    main()
