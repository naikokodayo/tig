#!/usr/bin/env python3
"""Real Git/PTY comparisons for diff wrapping, typed exports and safe errors."""
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
parser.add_argument('--before', action='store_true', help='verify the pre-fix wrapped export failure')
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
        return subprocess.run(['git', '-C', str(repo), *arguments], env=env, check=True, capture_output=True).stdout
    git('init', '-q')
    git('config', 'user.name', 'Export Tester')
    git('config', 'user.email', 'export@example.invalid')
    for content in ('old ' * 30, 'new ' * 30):
        (repo / 'file').write_text(content + '\n')
        git('add', 'file')
        git('-c', 'commit.gpgsign=false', 'commit', '-qm', 'Export long lines')
    original = git('rev-parse', 'HEAD').decode().strip()
    long_name = 'long_' + 'filename_' * 9 + '.py'
    for content in ('old', 'new'):
        (repo / long_name).write_text('def ' + 'function_' * 10 + '():\n' +
                                     '    context\n' * 10 + '    ' + content * 40 + '\n' +
                                     '    context\n' * 10)
        git('add', long_name)
        git('-c', 'commit.gpgsign=false', 'commit', '-qm', 'Long stat and hunk cells')
    long_patch = git('show', '--format=fuller', '--stat=120', '--patch', 'HEAD')
    git('reset', '--hard', original)
    config = directory / 'tigrc'
    steps = directory / 'steps'
    env.update(TIGRC_USER=str(config), TIG_SCRIPT=str(steps))
    for case, wrapped, width in [("ordinary", False, 40), ("wrapped", True, 40),
                                 ("wrapped-selected", True, 40), ("wrapped-in-view", True, 40),
                                 ("wrapped-existing", True, 40),
                                 ("split", False, 181), ("metadata", False, 80),
                                 ("custom", False, 80), ("collision", False, 80),
                                 ("runtime-color", False, 80), ("long-cells", True, 40),
                                 ("long-cells-selected", True, 40), ("long-cells-unwrapped", False, 40)]:
        if args.before and case not in ('wrapped', 'long-cells'):
            continue
        env["COLUMNS"] = str(width)
        config.write_text(f'set wrap-lines = {"yes" if wrapped else "no"}\n')
        if case in ('custom', 'collision'):
            with config.open('a') as file:
                file.write('color "' + ('custom input' if case == 'custom' else 'diff-stat') + '" red default\n')
        captures = {}
        for mode, binary in [('c', ROOT / 'src/tig'), ('rust', args.rust_binary.resolve())]:
            output = directory / f'{mode}-{case}.data'
            screen = directory / f'{mode}-{case}.screen'
            # Unsupported colors must not truncate an existing destination.
            if case in ('custom', 'collision', 'runtime-color', 'wrapped-existing'):
                output.write_text('untouched\n')
            steps.write_text((':enter\n' if case == 'split' else
                              ':move-last-line\n' if case in ('wrapped-selected', 'long-cells-selected') else
                              ':move-down\n' * 4 if case == 'wrapped-in-view' else
                              ':color "diff-stat" red default\n' if case == 'runtime-color' else '') +
                             f':save-display {screen}\n:save-view {output}\n:quit\n')
            command = [str(binary), '-C', str(repo), *(['log'] if case == 'split' else ['show', 'HEAD'])]
            if case in ('metadata', 'custom', 'collision', 'runtime-color'):
                source = directory / 'input'
                source.write_text('commit ' + 'a' * 40 + '\n' + ''.join(
                    prefix + 'value\n' for prefix in ('Author: ', 'Commit: ', 'Tagger: ', 'Date: ',
                                                       'AuthorDate: ', 'CommitDate: ', 'TaggerDate: ')) + ('custom input\n' if case == 'custom' else 'diff-stat metadata\n'))
                command = ['/bin/sh', '-c', 'exec "$1" -C "$2" show < "$3"',
                           'sh', str(binary), str(repo), str(source)]
            if case.startswith('long-cells'):
                source = directory / 'long-patch'
                source.write_bytes(long_patch)
                command = ['/bin/sh', '-c', 'exec "$1" -C "$2" show < "$3"',
                           'sh', str(binary), str(repo), str(source)]
            code, timed_out, transcript = h.terminal(command, env, 20)
            captures[mode] = {'exit_code': code, 'timed_out': timed_out,
                              'transcript': transcript, 'data': output.read_text() if output.exists() else '',
                              'screen': screen.read_text()}
            assert not timed_out, captures[mode]
        c, rust = captures['c'], captures['rust']
        assert c['exit_code'] == 0, c
        if case in ('custom', 'collision', 'runtime-color'):
            marker = 'custom input' if case == 'custom' else 'diff-stat metadata'
            lines = c['data'].splitlines()
            index = next(i for i, line in enumerate(lines) if f'text=[{marker}]' in line)
            expected_type = 'default' if case == 'runtime-color' else ''
            assert f'type={expected_type} selected=0' in lines[index - 1], (case, c)
            assert rust['exit_code'] != 0 and rust['data'] == 'untouched\n', rust
            assert 'save-view does not support custom color rules yet' in rust['transcript'], rust
        elif case == 'split':
            assert c['exit_code'] == rust['exit_code'] == 0, captures
            assert 'Prev: log\nParent: log\n' in c['data'], c
            assert rust['data'] == c['data'], captures
        elif case == 'wrapped-existing':
            assert rust['exit_code'] != 0 and rust['data'] == 'untouched\n', rust
            assert 'File exists' in rust['transcript'], rust
        elif case.startswith('long-cells'):
            assert f'text=[ {long_name} ]' in c['data'], c
            assert 'cells=2 text=[@@ ' in c['data'] and '[ def function_' in c['data'], c
            if args.before:
                assert rust['exit_code'] != 0 and not rust['data'], rust
            else:
                assert rust['exit_code'] == 0 and rust['data'] == c['data'], captures
                assert rust['screen'] == c['screen'], captures
        elif wrapped:
            assert 'line[  1] type=commit' in c['data'], c
            assert c['data'].count('type=diff-add selected=') == 4, c
            if args.before:
                assert rust['exit_code'] != 0 and not rust['data'], rust
                assert 'save-view does not support wrapped diff views yet' in rust['transcript'], rust
            else:
                assert rust['exit_code'] == 0 and rust['data'] == c['data'], captures
        else:
            assert rust['exit_code'] == 0, rust
            for prefix in ('Author: ', 'Commit: ', 'AuthorDate: ', 'CommitDate: '):
                lines = c['data'].splitlines()
                index = next(i for i, line in enumerate(lines) if f'text=[{prefix}' in line)
                assert 'type= selected=' in lines[index - 1], (prefix, lines)
            assert rust['data'] == c['data'], captures
        results[case] = captures
print(json.dumps({'source_commit': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
                  'mode': 'before' if args.before else 'after',
                  'binaries': {mode: {'path': str(binary), 'sha256': hashlib.sha256(binary.read_bytes()).hexdigest()}
                               for mode, binary in [('c', ROOT / 'src/tig'), ('rust', args.rust_binary.resolve())]},
                  'results': results}, indent=2))
