#!/usr/bin/env python3
"""Compare C and Rust argv classification and repeated command prompts on a real PTY."""
import importlib.util
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location('upstream', ROOT / 'rust/tests/upstream-suite.py')
upstream = importlib.util.module_from_spec(spec)
spec.loader.exec_module(upstream)

for binary in (ROOT / 'src/tig', ROOT / 'target/release/tig'):
    with tempfile.TemporaryDirectory(prefix='tig-command-variable-') as temporary:
        home = Path(temporary)
        repo = home / 'repo'
        repo.mkdir()
        env = upstream.environment([binary.parent, ROOT / 'test/tools'])
        env.update(HOME=temporary, TIGRC_SYSTEM='', TIGRC_USER=str(home / 'tigrc'),
                   TIG_SCRIPT=str(home / 'steps'), LINES='3', COLUMNS='80')
        subprocess.run(['git', 'init', '-q', str(repo)], env=env, check=True)
        for key, value in [('user.name', 'Test'), ('user.email', 'test@example.invalid')]:
            subprocess.run(['git', '-C', str(repo), 'config', key, value], env=env, check=True)
        (repo / 'file space;literal').write_text('content\n')
        subprocess.run(['git', '-C', str(repo), 'add', '.'], env=env, check=True)
        subprocess.run(['git', '-C', str(repo), 'commit', '-qm', 'base'], env=env, check=True)
        (home / 'tigrc').write_text(
            'bind generic 2 :!echo "%(prompt First: )" "%(prompt Second: )"\n')
        (home / 'steps').write_text(
            ':!echo %(fileargs)\n'
            f':save-display {home / "fileargs"}\n'
            ':view-close\n2\none<Enter>\ntwo<Enter>\n'
            f':save-display {home / "prompts"}\n:quit\n')
        code, timed_out, transcript = upstream.terminal(
            [str(binary), '-C', str(repo), 'file space;literal'], env, 15)
        assert code == 0 and not timed_out, (binary, code, transcript)
        for name, expected in [('fileargs', 'file space;literal'), ('prompts', 'one two')]:
            screen = (home / name).read_text()
            assert screen.splitlines()[0] == expected, (binary, name, screen)
        assert not (repo / 'literal').exists(), binary
        print(f'PASS: {binary.parent.name}/tig classifies literal filename and consumes both prompts')
