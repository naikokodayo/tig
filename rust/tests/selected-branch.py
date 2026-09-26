#!/usr/bin/env python3
"""Paired real-PTY checks of selected-commit reference priority and argv boundaries."""
import importlib.util
import json
import sys
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location('upstream', ROOT / 'rust/tests/upstream-suite.py')
upstream = importlib.util.module_from_spec(spec)
spec.loader.exec_module(upstream)

with tempfile.TemporaryDirectory(prefix='tig-selected-branch-') as temporary:
    home = Path(temporary)
    repo = home / 'repo'
    env = upstream.environment([])
    env.update(HOME=temporary, GIT_CONFIG_GLOBAL='/dev/null', TIGRC_SYSTEM='',
               TIGRC_USER=str(home / 'tigrc'), TIG_SCRIPT=str(home / 'steps'),
               GIT_AUTHOR_NAME='Test', GIT_AUTHOR_EMAIL='test@example.invalid',
               GIT_COMMITTER_NAME='Test', GIT_COMMITTER_EMAIL='test@example.invalid')

    def git(*args):
        return subprocess.check_output(['git', '-C', str(repo), *args], env=env).decode().strip()

    subprocess.run(['git', 'init', '-q', str(repo)], env=env, check=True)
    git('commit', '--allow-empty', '-qm', 'selected')
    selected = git('rev-parse', 'HEAD')
    git('commit', '--allow-empty', '-qm', 'unrelated checkout')
    git('branch', '-m', 'checked-out')
    git('remote', 'add', 'upstream', '/unused')
    git('config', 'branch.checked-out.remote', 'upstream')
    git('config', 'branch.checked-out.merge', 'refs/heads/tracked')
    (home / 'tigrc').write_text('set show-changes = no\nset refresh-mode = manual\n')
    (home / 'capture.py').write_text(
        'import json, pathlib, sys\n'
        f'pathlib.Path({str(home / "argv")!r}).write_text(json.dumps(sys.argv[1:]))\n')
    cases = [
        ('local-priority', ['refs/heads/r1.0', 'refs/heads/r1.1.2', 'refs/heads/r1.1.x',
                            'refs/remotes/upstream/tracked', 'refs/tags/v1'],
         'r1.1.2|v1|upstream|r1.1.2|'),
        ('remote-priority', ['refs/remotes/upstream/aaa', 'refs/remotes/upstream/tracked'],
         'tracked||upstream|upstream/tracked|'),
        ('unconfigured-remote', ['refs/remotes/custom/topic'], 'topic||custom|custom/topic|'),
        ('tag-only', ['refs/tags/v1'], '|v1|origin|v1|'),
        ('ambiguous-tag', ['refs/tags/v1'], '|refs/tags/v1|origin|refs/tags/v1|'),
        ('replacement-only', [], '||origin|replaced|'),
        ('replacement-with-refs', ['refs/heads/aaa', 'refs/heads/bbb', 'refs/tags/v1'], '|v1|origin|v1|'),
        ('no-refs', [], '||origin||'),
        ('detached-head', [], 'HEAD||origin|HEAD|'),
        ('replacement-detached', [], '||origin|HEAD|'),
        ('literal-branch', ['refs/heads/topic;literal'], 'topic;literal||origin|topic;literal|'),
    ]
    for name, references, expected in cases:
        checkout = ['--detach', selected] if name in ('detached-head', 'replacement-detached') else ['checked-out']
        git('checkout', '-q', *checkout)
        if name == 'ambiguous-tag':
            git('branch', 'v1', 'checked-out')
        for reference in references:
            git('update-ref', reference, selected)
        if name.startswith('replacement-'):
            git('replace', selected, git('commit-tree', selected + '^{tree}', '-m', 'replacement'))
        for binary in (ROOT / 'src/tig', ROOT / 'target/release/tig'):
            output = home / 'argv'
            output.unlink(missing_ok=True)
            (home / 'steps').write_text(
                f':exec @{sys.executable} {home / "capture.py"} '
                '"branch=%(branch)" "tag=%(tag)" "remote=%(remote)" "refname=%(refname)"\n:quit\n')
            code, timed_out, transcript = upstream.terminal(
                [str(binary), '-C', str(repo), selected], env, 15)
            assert code == 0 and not timed_out, (name, binary, code, transcript)
            actual = json.loads(output.read_text())
            assert actual == [f'{key}={value}' for key, value in zip(('branch', 'tag', 'remote', 'refname'), expected.split('|'))], (name, binary, actual, expected)
            print(f'PASS: {binary.parent.name}/tig {name}', flush=True)
        for reference in references:
            git('update-ref', '-d', reference)
        if name == 'ambiguous-tag':
            git('branch', '-D', 'v1')
        if name.startswith('replacement-'):
            git('replace', '-d', selected)
