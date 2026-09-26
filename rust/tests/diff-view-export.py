#!/usr/bin/env python3
"""Real C/Rust save-view headers and fail-closed wrapped export regression."""
import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location('suite', ROOT / 'rust/tests/upstream-suite.py')
h = importlib.util.module_from_spec(spec)
spec.loader.exec_module(h)
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--rust-binary', type=Path, default=ROOT / 'target/release/tig')
parser.add_argument('--before', action='store_true', help='verify the three pre-fix failures')
args = parser.parse_args()
results = {}
with tempfile.TemporaryDirectory(prefix='tig-view-export-') as temporary:
    directory = Path(temporary)
    repo = directory / 'repo'
    repo.mkdir()
    env = h.environment([])
    env.update(GIT_CONFIG_GLOBAL='/dev/null', TIGRC_SYSTEM=str(ROOT / 'tigrc'), LC_ALL='C',
               GIT_AUTHOR_DATE='2020-01-01T00:00:00+0000', GIT_COMMITTER_DATE='2020-01-01T00:00:00+0000',
               COLUMNS='40', LINES='30')
    def git(*arguments):
        subprocess.run(['git', '-C', str(repo), *arguments], env=env, check=True, capture_output=True)
    git('init', '-q')
    git('config', 'user.name', 'Export Tester')
    git('config', 'user.email', 'export@example.invalid')
    for content in ('old ' * 30, 'new ' * 30):
        (repo / 'file').write_text(content + '\n')
        git('add', 'file')
        git('-c', 'commit.gpgsign=false', 'commit', '-qm', 'Export long lines')
    config = directory / 'tigrc'
    steps = directory / 'steps'
    env.update(TIGRC_USER=str(config), TIG_SCRIPT=str(steps))
    for case, wrapped, width in [("ordinary", False, 40), ("wrapped", True, 40), ("split", False, 181)]:
        env["COLUMNS"] = str(width)
        config.write_text(f'set wrap-lines = {"yes" if wrapped else "no"}\n')
        captures = {}
        for mode, binary in [('c', ROOT / 'src/tig'), ('rust', args.rust_binary.resolve())]:
            output = directory / f'{mode}-{case}.data'
            # Refusing wrapped export must not truncate an existing destination.
            output.write_text('untouched\n')
            steps.write_text((':enter\n' if case == 'split' else '') + f':save-view {output}\n:quit\n')
            code, timed_out, transcript = h.terminal([str(binary), '-C', str(repo), *(['log'] if case == 'split' else ['show', 'HEAD'])], env, 20)
            captures[mode] = {'exit_code': code, 'timed_out': timed_out,
                              'transcript': transcript, 'data': output.read_text()}
            assert not timed_out, captures[mode]
        c, rust = captures['c'], captures['rust']
        assert c['exit_code'] == 0, c
        if case == 'split':
            assert c['exit_code'] == rust['exit_code'] == 0, captures
            assert 'Prev: log\nParent: log\n' in c['data'], c
            if args.before:
                assert 'Prev: log\n' not in rust['data'] and 'Parent: log\n' in rust['data'], rust
            else:
                assert rust['data'] == c['data'], captures
        elif wrapped:
            assert 'line[  1] type=commit' in c['data'], c
            assert c['data'].count('type=diff-add selected=') >= 2, c
            if args.before:
                assert rust['exit_code'] == 0 and rust['data'] != c['data'], captures
                assert 'line[  1] type=default' in rust['data'], rust
            else:
                assert rust['exit_code'] != 0, rust
                assert 'save-view does not support wrapped diff views yet' in rust['transcript'], rust
                assert rust['data'] == 'untouched\n', rust
        else:
            assert rust['exit_code'] == 0, rust
            for prefix in ('Author: ', 'Commit: ', 'AuthorDate: ', 'CommitDate: '):
                lines = c['data'].splitlines()
                index = next(i for i, line in enumerate(lines) if f'text=[{prefix}' in line)
                assert 'type= selected=' in lines[index - 1], (prefix, lines)
            if args.before:
                assert rust['data'] != c['data'] and 'type=default' in rust['data'], captures
            else:
                assert rust['data'] == c['data'], captures
        results[case] = captures
print(json.dumps({'source_commit': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
                  'mode': 'before' if args.before else 'after',
                  'binaries': {mode: {'path': str(binary), 'sha256': hashlib.sha256(binary.read_bytes()).hexdigest()}
                               for mode, binary in [('c', ROOT / 'src/tig'), ('rust', args.rust_binary.resolve())]},
                  'results': results}, indent=2))
