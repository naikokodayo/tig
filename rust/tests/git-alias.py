#!/usr/bin/env python3
"""Real Git shell-alias/worktree startup and path-boundary regressions."""
import importlib.util
import os
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location('upstream', ROOT / 'rust/tests/upstream-suite.py')
upstream = importlib.util.module_from_spec(spec)
spec.loader.exec_module(upstream)

with tempfile.TemporaryDirectory(prefix='tig-alias-') as temporary:
    home = Path(temporary)
    base = home / 'base'
    worktree = home / 'worktree'
    env = upstream.environment([])
    env.update(HOME=temporary, GIT_CONFIG_GLOBAL='/dev/null', TIGRC_SYSTEM='',
               TIGRC_USER=str(home / 'tigrc'), TIG_SCRIPT=str(home / 'steps'),
               GIT_AUTHOR_NAME='Test', GIT_AUTHOR_EMAIL='test@example.invalid',
               GIT_COMMITTER_NAME='Test', GIT_COMMITTER_EMAIL='test@example.invalid')

    def git(*args):
        return subprocess.check_output(['git', '-C', str(base), *args], env=env).decode().strip()

    subprocess.run(['git', 'init', '-q', str(base)], env=env, check=True)
    (base / 'root-file').write_text('root\n')
    git('add', '.')
    git('commit', '-qm', 'root-only')
    for name in ('sub dir', '子目录', '-leading'):
        directory = base / name
        directory.mkdir()
        for filename in ('file name', '文件', '-option'):
            (directory / filename).write_text('content\n')
    git('add', '.')
    git('commit', '-qm', 'path-match')
    git('worktree', 'add', '-q', '-b', 'linked', str(worktree))
    subprocess.run(['git', '-C', str(worktree), 'commit', '--allow-empty', '-qm',
                    'linked-only'], env=env, check=True)
    (home / 'tigrc').write_text('set show-changes = no\nset refresh-mode = manual\n')
    screen = home / 'screen'
    (home / 'steps').write_text(f':save-display {screen}\n:quit\n')
    count = 0
    for binary in (ROOT / 'src/tig', ROOT / 'target/release/tig'):
        routed = dict(env, PATH=str(binary.parent) + os.pathsep + env['PATH'])
        for repository in (base, worktree):
            for directory in ('sub dir', '子目录', '-leading'):
                cases = [([], True), (['--', 'file name'], False),
                         (['file name'], False), (['--', '文件'], False),
                         (['--max-count=1', '--', '-option'], False),
                         (['--', '../root-file'], True)]
                for args, root_expected in cases:
                    for alias in (False, True):
                        command = (['git', '-C', str(repository / directory), '-c', 'alias.tig=!tig', 'tig']
                                   if alias else [str(binary), '-C', str(repository / directory)])
                        screen.unlink(missing_ok=True)
                        code, timed_out, transcript = upstream.terminal(command + args, routed, 15)
                        # C resolves implicit path arguments before restoring the alias cwd.
                        # Record that inherited limitation, rather than claim C parity.
                        if binary == ROOT / 'src/tig' and alias and args == ['file name']:
                            assert code == 1 and not timed_out and 'No revisions match' in transcript, transcript
                            print('C LIMITATION: implicit alias path rejected:', directory, repository.name, flush=True)
                            continue
                        assert code == 0 and not timed_out, (command, args, code, transcript)
                        text = screen.read_text()
                        if not args:
                            assert ('linked-only' in text) == (repository == worktree), text
                        assert ('root-only' in text) == root_expected, (command, args, text)
                        assert ('path-match' in text) == (args != ['--', '../root-file']), (command, args, text)
                        count += 1
        print(f'PASS {binary}: normal/linked worktree, direct/alias, spaces/Unicode/options/parent paths', flush=True)

    # Invalid inherited prefixes must fail before scripts or config run. Unlike
    # C's unchecked chdir, Rust intentionally rejects lexical and symlink escape.
    (base / 'outside').symlink_to(home, target_is_directory=True)
    for prefix in ('../', str(home), 'outside/', 'missing/'):
        screen.unlink(missing_ok=True)
        invalid = dict(env, GIT_PREFIX=prefix)
        code, timed_out, transcript = upstream.terminal(
            [str(ROOT / 'target/release/tig'), '-C', str(base)], invalid, 15)
        assert code != 0 and not timed_out and not screen.exists(), (prefix, code, transcript)
        count += 1
    print(f'PASS {count} real-PTY cases; invalid prefixes rejected before display', flush=True)
