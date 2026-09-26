#!/usr/bin/env python3
"""C/Rust regression: filtered main history keeps auto topological order."""
import importlib.util
import json
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location('upstream', ROOT / 'rust/tests/upstream-suite.py')
upstream = importlib.util.module_from_spec(spec)
spec.loader.exec_module(upstream)

with tempfile.TemporaryDirectory(prefix='tig-commit-order-') as temporary:
    directory = Path(temporary)
    repo = directory / 'repo'
    repo.mkdir()
    subprocess.run(['tar', '-xzf', str(ROOT / 'test/main/commit-order-edge-case-test.tgz'),
                    '-C', str(repo)], check=True)
    subprocess.run(['git', '-C', str(repo), 'reset', '-q', '--hard'], check=True)
    config = directory / 'tigrc'
    config.write_text('set show-changes = no\nset line-graphics = utf-8\n'
                      'set main-view = date author commit-title:yes,graph\n')
    env = upstream.environment([])
    env.update(HOME=str(directory), GIT_CONFIG_GLOBAL='/dev/null', TIGRC_SYSTEM='',
               TIGRC_USER=str(config), LINES='10')
    results = {}
    for mode, binary in [('c', ROOT / 'src/tig'), ('rust', ROOT / 'target/release/tig')]:
        screen = directory / f'{mode}.screen'
        steps = directory / 'steps'
        steps.write_text(f':save-display {screen}\n:quit\n')
        code, timeout, transcript = upstream.terminal(
            [str(binary), '-C', str(repo), '--no-merges'], {**env, 'TIG_SCRIPT': str(steps)}, 10)
        assert code == 0 and not timeout, (mode, code, timeout, transcript)
        lines = [line for line in screen.read_text().splitlines()
                 if 'More featuresA' in line or 'More master' in line]
        results[mode] = lines
    print(json.dumps(results, ensure_ascii=False))
    for mode, lines in results.items():
        assert len(lines) == 2 and 'More featuresA' in lines[0] and 'More master' in lines[1], (mode, lines)
