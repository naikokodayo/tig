#!/usr/bin/env python3
"""Run after cargo build --release: supplied diff input survives redraw without Git."""
import os
from pathlib import Path
import subprocess
import tempfile

BINARY = Path(__file__).resolve().parents[2] / 'target/release/tig'
with tempfile.TemporaryDirectory(prefix='tig-diff-input-') as temporary:
    directory = Path(temporary)  # Deliberately outside a repository.
    script = directory / 'steps'
    script.write_text(':refresh\n:save-display screen\n')
    environment = {k: v for k, v in os.environ.items()
                   if not k.startswith(('TIG', 'GIT_', 'XDG_'))}
    environment.update(TIGRC_SYSTEM='', TIGRC_USER='/dev/null', TIG_SCRIPT=str(script),
                       COLUMNS='100', LINES='8', LC_ALL='C')
    cases = [
        ('commit ' + 'a' * 40 + '\n\n    supplied diff\n', 'a' * 40),
        ('commit ' + 'b' * 64 + '\n\n    supplied diff\n', 'b' * 64),
        ('    commit ' + 'c' * 40 + '\n', 'HEAD'),
        ('\x1b[31mcontent\n', 'HEAD'),
        ('', 'HEAD'),
    ]
    for raw, revision in cases:
        result = subprocess.run([str(BINARY), 'show'], input=raw, text=True,
                                capture_output=True, cwd=directory, env=environment, timeout=10)
        assert result.returncode == 0 and not result.stderr, result.stderr
        screen = (directory / 'screen').read_text()
        assert f'[diff] {revision}' in screen, screen
        assert '\x1b' not in screen, screen
        if raw:
            assert screen.splitlines()[0] == raw.splitlines()[0].replace('\x1b', r'\x1b'), screen
    result = subprocess.run([str(BINARY), 'show', '--stdin'], input='HEAD\n', text=True,
                            capture_output=True, cwd=directory, env=environment, timeout=10)
    assert result.returncode != 0 and 'Forwarding revision input' in result.stderr, result
print(f'{len(cases) + 1} diff input checks passed')
