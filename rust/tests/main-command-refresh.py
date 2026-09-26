#!/usr/bin/env python3
"""Commands in fullscreen grep must preserve both grep and cached main context."""
import importlib.util
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location('upstream', ROOT / 'rust/tests/upstream-suite.py')
upstream = importlib.util.module_from_spec(spec)
spec.loader.exec_module(upstream)

with tempfile.TemporaryDirectory(prefix='tig-main-command-refresh-') as temporary:
    repo = Path(temporary)
    env = upstream.environment([])
    env.update(GIT_AUTHOR_NAME='Test', GIT_AUTHOR_EMAIL='test@example.invalid',
               GIT_COMMITTER_NAME='Test', GIT_COMMITTER_EMAIL='test@example.invalid',
               HOME=str(repo), GIT_CONFIG_GLOBAL='/dev/null', TIGRC_SYSTEM='',
               TIGRC_USER='/dev/null', TIG_SCRIPT=str(repo / 'steps'))
    subprocess.run(['git', 'init', '-q', str(repo)], env=env, check=True)
    (repo / 'file').write_text('needle\n')
    for args in [('add', 'file'), ('commit', '-qm', 'base')]:
        subprocess.run(['git', '-C', str(repo), *args], env=env, check=True)
    for command in (':exec @echo fine', ':!echo fine'):
        for name in ('grep', 'main'):
            (repo / name).unlink(missing_ok=True)
        close_pager = ':view-close\n' if command.startswith(':!') else ''
        (repo / 'steps').write_text(
            ':g\nneedle\n' + command + '\n' + close_pager
            + f':save-display {repo / "grep"}\n:back\n'
            + f':save-display {repo / "main"}\n:quit\n')
        code, timed_out, transcript = upstream.terminal(
            [str(ROOT / 'target/release/tig'), '-C', str(repo)], env, 10)
        assert code == 0 and not timed_out, (command, code, transcript)
        grep_screen = (repo / 'grep').read_text()
        main_screen = (repo / 'main').read_text()
        assert '[grep]' in grep_screen and 'needle' in grep_screen, grep_screen
        assert '[main]' in main_screen and 'base' in main_screen, main_screen
        print(f'PASS: {command} preserves fullscreen grep and cached main')
