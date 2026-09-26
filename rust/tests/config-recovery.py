#!/usr/bin/env python3
"""C/Rust PTY regression for diagnosed date recovery and prefixed legacy colors.

Run after make src/tig and cargo build --release. Uses only the stdlib.
"""
import hashlib
import json
import os
from pathlib import Path
import runpy
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]
terminal = runpy.run_path(str(ROOT / 'rust/tests/upstream-suite.py'))['terminal']
results = []
with tempfile.TemporaryDirectory(prefix='tig-config-recovery-') as temporary:
    directory = Path(temporary)
    terminal.__globals__['ROOT'] = directory
    env = {k: v for k, v in os.environ.items() if not k.startswith(('TIG', 'GIT_', 'XDG_'))}
    env.update(HOME=temporary, GIT_CONFIG_NOSYSTEM='1', GIT_CONFIG_GLOBAL='/dev/null',
               GIT_AUTHOR_NAME='Fixture', GIT_AUTHOR_EMAIL='fixture@example.invalid',
               GIT_COMMITTER_NAME='Fixture', GIT_COMMITTER_EMAIL='fixture@example.invalid',
               GIT_AUTHOR_DATE='1440961292 +0900', GIT_COMMITTER_DATE='1440961292 +0900',
               TERM='xterm', LC_ALL='en_US.UTF-8', TZ='UTC', COLUMNS='80', LINES='5',
               TIGRC_SYSTEM='', TIGRC_USER=str(directory / 'tigrc'),
               TIG_SCRIPT=str(directory / 'steps'))
    subprocess.run(['git', 'init', '-q'], cwd=directory, env=env, check=True)
    subprocess.run(['git', 'commit', '-qm', 'subject', '--allow-empty'], cwd=directory, env=env, check=True)
    for suffix in ('date', 'date-display'):
        for value in ('local', 'short', 'LOCAL', 'invalid'):
            (directory / 'tigrc').write_text(
                'set show-changes = no\n'
                'set main-view = date:custom,format=%Y commit-title:yes,graph=no,refs=no\n'
                f'set main-view-{suffix} = {value}\n'
                'color tree.tree-head yellow default\n'
                'color diff.tree-head cyan default\n'
                'color main.main-revgraph yellow default\n')
            (directory / 'steps').write_text(':save-display screen\n:save-options saved\n:quit\n')
            for binary in (ROOT / 'src/tig', ROOT / 'target/release/tig'):
                for name in ('screen', 'saved'):
                    (directory / name).unlink(missing_ok=True)
                code, timeout, transcript = terminal([str(binary)], env, 15)
                screen = (directory / 'screen').read_text() if (directory / 'screen').exists() else ''
                saved = (directory / 'saved').read_text() if (directory / 'saved').exists() else ''
                passed = (code == 0 and not timeout
                          and (screen.splitlines() or [''])[0].rstrip() == '2015-08-31 04:01 +0900 subject'
                          and f"'{value}' is no longer supported for date-display" in transcript
                          and 'tree-head has been replaced by tree.header' in transcript
                          and 'main.main-revgraph is obsolete' in transcript
                          and ['color', 'tree.header', 'yellow', 'default'] in [line.split() for line in saved.splitlines()]
                          and ['color', 'diff.header', 'cyan', 'default'] in [line.split() for line in saved.splitlines()]
                          and 'main.main-revgraph' not in saved)
                results.append(dict(binary=str(binary.relative_to(ROOT)), suffix=suffix, value=value,
                                    passed=passed, code=code, timeout=timeout, screen=screen,
                                    transcript=transcript, saved_colors=[line for line in saved.splitlines()
                                    if line.startswith(('color tree.header', 'color diff.header'))]))
report = dict(source_commit=subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
              binaries={name: hashlib.sha256((ROOT / name).read_bytes()).hexdigest()
                        for name in ('src/tig', 'target/release/tig')}, results=results)
output = Path(os.environ.get('TIG_RECOVERY_EVIDENCE', ROOT / 'migration/evidence/config-render-review-recovery.json'))
output.write_text(json.dumps(report, indent=2, ensure_ascii=False) + '\n')
assert all(r['passed'] for r in results), [(r['binary'], r['suffix'], r['value']) for r in results if not r['passed']]
print(f'{len(results)} C/Rust recovery checks passed: {output}')
