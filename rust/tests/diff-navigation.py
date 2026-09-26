#!/usr/bin/env python3
"""Paired C/Rust regression for stat widths when navigating a maximized split."""
import hashlib
import importlib.util
import json
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location('upstream_suite', ROOT / 'rust/tests/upstream-suite.py')
harness = importlib.util.module_from_spec(spec)
spec.loader.exec_module(harness)
results = []
with tempfile.TemporaryDirectory(prefix='tig-diff-navigation-') as temporary:
    directory = Path(temporary)
    repo = directory / 'repo'
    repo.mkdir()
    env = harness.environment([])
    env.update(GIT_CONFIG_GLOBAL='/dev/null', TIGRC_SYSTEM='', LC_ALL='C')
    def git(*args):
        subprocess.run(['git', '-C', str(repo), *args], env=env, check=True, capture_output=True)
    git('init', '-q')
    git('config', 'user.name', 'Test')
    git('config', 'user.email', 'test@example.invalid')
    filename = 'long-directory/' + 'long-file-name-' * 6 + '.txt'
    (repo / filename).parent.mkdir()
    for index in range(3):
        (repo / filename).write_text('line\n' * (index + 1))
        git('add', '.')
        git('-c', 'commit.gpgsign=false', 'commit', '-qm', f'commit {index}')
    config = directory / 'tigrc'
    config.write_text('set vertical-split = vertical\nset line-graphics = ascii\n')
    env['TIGRC_USER'] = str(config)
    for width in (180, 181):
        captured = {}
        for mode, binary in [('c', ROOT / 'src/tig'), ('rust', ROOT / 'target/release/tig')]:
            paths = {name: directory / f'{mode}-{width}-{name}.screen'
                     for name in ('split', 'next', 'previous', 'refresh')}
            script = directory / 'steps'
            script.write_text(f':enter\n:save-display {paths["split"]}\n'
                              f':maximize\n:next\n:save-display {paths["next"]}\n'
                              f':previous\n:save-display {paths["previous"]}\n'
                              f':refresh\n:save-display {paths["refresh"]}\n:quit\n')
            run_env = {**env, 'TIG_SCRIPT': str(script), 'COLUMNS': str(width), 'LINES': '30'}
            code, timeout, transcript = harness.terminal([str(binary), '-C', str(repo)], run_env, 10)
            assert code == 0 and not timeout, (mode, code, transcript)
            captured[mode] = {}
            for state, path in paths.items():
                rows = path.read_text().splitlines()
                if state == 'split':
                    rows = [row[width - width // 2 + 1:] for row in rows]
                stats = [row.rstrip() for row in rows if ' | ' in row]
                assert len(stats) == 1, (mode, state, rows)
                captured[mode][state] = stats[0]
        for state in paths:
            assert captured['rust'][state] == captured['c'][state], (width, state, captured)
            if state != 'split':
                assert filename in captured['rust'][state], (width, state, captured)
        results.append({'width': width, 'stats': captured})
print(json.dumps({'source_commit': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
                  'binary_sha256': {str(p.relative_to(ROOT)): hashlib.sha256(p.read_bytes()).hexdigest()
                                    for p in (ROOT / 'src/tig', ROOT / 'target/release/tig')},
                  'results': results}, indent=2))
