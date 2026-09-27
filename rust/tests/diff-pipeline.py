#!/usr/bin/env python3
"""Pair C and Rust for word-diff precedence and filtered decorated commits."""
import importlib.util
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location('suite', ROOT / 'rust/tests/upstream-suite.py')
h = importlib.util.module_from_spec(spec)
spec.loader.exec_module(h)
with tempfile.TemporaryDirectory(prefix='tig-diff-pipeline-') as temporary:
    directory = Path(temporary)
    repo = directory / 'repo'
    repo.mkdir()
    env = h.environment([])
    env.update(GIT_CONFIG_GLOBAL='/dev/null', TIGRC_SYSTEM='', LC_ALL='C', TERM='dumb',
               COLUMNS='80', LINES='30', GIT_AUTHOR_DATE='2020-01-01T00:00:00+0000',
               GIT_COMMITTER_DATE='2020-01-01T00:00:00+0000')

    def git(*args):
        subprocess.run(['git', '-C', str(repo), *args], env=env, check=True, capture_output=True)

    git('init', '-q')
    git('config', 'user.name', 'Test')
    git('config', 'user.email', 'test@example.invalid')
    for content in ('old line\n', 'new line\n'):
        (repo / 'file').write_text(content)
        git('add', 'file')
        git('-c', 'commit.gpgsign=false', 'commit', '-qm', 'changed file')

    cases = {
        'inferred-word': 'set word-diff = no\nset diff-options = --word-diff=plain\nset diff-highlight = wc\n',
        'forced-word': 'set word-diff = yes\nset diff-options = --word-diff=none\nset diff-highlight = wc\n',
        'decorated-filter': 'set diff-highlight = wc\n',
        'cli-word-override': 'set diff-options = --word-diff=plain\nset diff-highlight = wc\n',
    }
    for name, config in cases.items():
        (directory / 'tigrc').write_text(config)
        screens = {}
        for mode, binary in [('c', ROOT / 'src/tig'), ('rust', ROOT / 'target/release/tig')]:
            screen = directory / f'{name}-{mode}.screen'
            (directory / 'steps').write_text(f':save-display {screen}\n:quit\n')
            run_env = {**env, 'TIGRC_USER': str(directory / 'tigrc'),
                       'TIG_SCRIPT': str(directory / 'steps')}
            args = ['--word-diff=none'] if name == 'cli-word-override' else []
            code, timed_out, transcript = h.terminal(
                [str(binary), '-C', str(repo), 'show', 'HEAD', *args], run_env, 10)
            assert code == 0 and not timed_out, (name, mode, code, transcript)
            screens[mode] = screen.read_text()
        assert screens['rust'] == screens['c'], (name, screens)
        if name in ('decorated-filter', 'cli-word-override'):
            assert 'Refs:' not in screens['c'] and '[diff] HEAD - line 1 of 1' in screens['c']
        else:
            assert '[-old-]{+new+} line' in screens['c'], (name, screens['c'])
print(f'{len(cases)} paired diff pipeline checks passed')
