#!/usr/bin/env python3
"""Real C/Rust PTY checks for optioned grep revisions and lossless blob targets."""
import argparse
import importlib.util
import json
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location('upstream_suite', ROOT / 'rust/tests/upstream-suite.py')
harness = importlib.util.module_from_spec(spec)
spec.loader.exec_module(harness)
parser = argparse.ArgumentParser()
parser.add_argument('--rust-binary', type=Path, default=ROOT / 'target/release/tig')
args = parser.parse_args()
checks = []
with tempfile.TemporaryDirectory(prefix='tig-grep-revisions-') as temporary:
    directory = Path(temporary)
    repo = directory / 'repo'
    repo.mkdir()
    env = harness.environment([])
    env.update(GIT_CONFIG_GLOBAL='/dev/null', TIGRC_SYSTEM='', LC_ALL='C')
    def git(*arguments):
        return subprocess.run(['git', '-C', str(repo), *arguments], env=env,
                              check=True, capture_output=True).stdout
    git('init', '-q')
    git('config', 'user.name', 'Test')
    git('config', 'user.email', 'test@example.invalid')
    (repo / 'file.txt').write_text('first\nNEEDLE committed\nlast\n')
    (repo / 'sub').mkdir()
    (repo / 'sub/file.txt').write_text('nested\nNEEDLE nested committed\nend\n')
    (repo / 'HEAD:literal').write_text('NEEDLE colon worktree\n')
    (repo / 'literal:colon').write_text('NEEDLE implicit colon worktree\n')
    (repo / 'odd\nname').write_text('NEEDLE newline worktree\n')
    git('add', '.')
    git('-c', 'commit.gpgsign=false', 'commit', '-qm', 'base')
    (repo / 'context.txt').write_text('zero\nNEEDLE one\ntwo\nthree\nfour\nfive\nsix\nNEEDLE seven\neight\n')
    (repo / '--\nfile').write_text('before\nNEEDLE separator path\nafter\n')
    git('add', '.')
    git('-c', 'commit.gpgsign=false', 'commit', '-qm', 'context fixtures')
    (repo / 'untracked.txt').write_text('untracked\nNEEDLE new file\nend\n')
    (repo / 'file.txt').write_text('first\nNEEDLE staged\nlast\n')
    git('add', 'file.txt')
    (repo / 'file.txt').write_text('WRONG worktree\n')
    (repo / 'sub/file.txt').write_text('WRONG nested worktree\n')
    patterns = directory / 'patterns'
    patterns.write_text('NEEDLE\n')
    (repo / 'e').write_text('NEEDLE\n')
    config = directory / 'tigrc'
    config.write_text('set vertical-split = no\nset line-graphics = ascii\n')
    env['TIGRC_USER'] = str(config)
    def run(mode, arguments, enter=True):
        binary = ROOT / 'src/tig' if mode == 'c' else args.rust_binary.resolve()
        screen = directory / f'{mode}.screen'
        blob = directory / f'{mode}.blob'
        screen.unlink(missing_ok=True)
        blob.unlink(missing_ok=True)
        script = directory / 'steps'
        refreshed = directory / f'{mode}.refresh'
        refreshed.unlink(missing_ok=True)
        script.write_text(f':save-display {screen}\n' +
                          (f':2\n:enter\n:save-display {blob}\n' + (f':refresh\n:save-display {refreshed}\n' if mode == 'rust' else '') if enter else '') + ':quit\n')
        code, timeout, transcript = harness.terminal(
            [str(binary), '-C', str(repo), 'grep', *arguments],
            {**env, 'TIG_SCRIPT': str(script), 'COLUMNS': '100', 'LINES': '20'}, 10)
        assert code == 0 and not timeout and screen.exists(), (mode, arguments, code, transcript)
        if enter and mode == 'rust':
            assert blob.read_bytes() == refreshed.read_bytes(), (arguments, 'blob refresh changed source')
        return screen.read_bytes(), blob.read_bytes() if enter else b''
    cases = [
        ['-inw', 'needle', 'HEAD', '--', 'file.txt'],
        ['-inFeNEEDLE', 'HEAD', '--', 'file.txt'],
        ['-infe', 'HEAD', '--', 'file.txt'],
        ['-inf', str(patterns), 'HEAD', '--', 'file.txt'],
        ['-inm1', 'needle', 'HEAD', '--', 'file.txt'],
        ['-i', 'needle', 'HEAD', '--', 'file.txt'],
        ['-e', 'NEEDLE', 'HEAD', '--', 'file.txt'],
        ['-F', '-eNEEDLE', 'HEAD', '--', 'file.txt'],
        ['-f', str(patterns), 'HEAD', '--', 'file.txt'],
        ['-e', 'HEAD', '-e', 'NEEDLE', 'HEAD', '--', 'file.txt'],
        ['-i', 'needle', 'HEAD^{tree}', '--', 'file.txt'],
        ['-i', 'needle', 'HEAD', 'HEAD^{tree}', '--', 'file.txt'],
        ['-i', 'needle', 'HEAD', 'file.txt'],
    ]
    for arguments in cases:
        c = run('c', arguments)
        rust = run('rust', arguments)
        assert c == rust, (arguments, c, rust)
        assert b'NEEDLE committed' in rust[1] and b'WRONG worktree' not in rust[1]
        checks.append({'argv': arguments, 'route': 'paired screens and selected revision blob', 'pass': True})
    for flags in [['-inA1'], ['-inB', '1'], ['-inC1'], ['-in1'], ['-inm1', '-C2'], ['-A1'], ['-B', '1'], ['--context=1'], ['-1'], ['-m1', '-C2']]:
        arguments = [*flags, '-e', 'NEEDLE', 'HEAD', '--', 'context.txt']
        c = run('c', arguments)
        rust = run('rust', arguments)
        assert c == rust, (arguments, c, rust)
        checks.append({'argv': arguments, 'route': 'paired context screens and blob', 'pass': True})
    # C splits at the first colon, so these are Rust safety oracles, not parity claims.
    for arguments, expected in [
        (['--regexp=NEEDLE', 'HEAD', '--', 'file.txt'], b'NEEDLE committed'),
        (['--file=' + str(patterns), 'HEAD', '--', 'file.txt'], b'NEEDLE committed'),
        (['-i', 'needle', 'HEAD:sub'], b'NEEDLE nested committed'),
        (['-e', 'NEEDLE', '--', 'HEAD:literal'], b'NEEDLE colon worktree'),
        (['-e', 'NEEDLE', '--', 'odd\nname'], b'NEEDLE newline worktree'),
        (['-e', 'HEAD', '-e', 'NEEDLE', '--', 'HEAD:literal'], b'NEEDLE colon worktree'),
        (['-i', 'needle', 'literal:colon'], b'NEEDLE implicit colon worktree'),
        (['--cached', '-C1', '-e', 'NEEDLE', '--', 'file.txt'], b'NEEDLE staged'),
        (['--cached', '-inC1', '-e', 'NEEDLE', '--', 'file.txt'], b'NEEDLE staged'),
        (['--untracked', '-A1', '-e', 'NEEDLE', '--', 'untracked.txt'], b'NEEDLE new file'),
        (['-C1', '-e', 'NEEDLE', '--', '--\nfile'], b'NEEDLE separator path'),
        (['-inC1', '-e', 'NEEDLE', '--', '--\nfile'], b'NEEDLE separator path'),
        (['-e', 'NEEDLE', '--and', '--not', '-e', 'missing', 'HEAD:sub'], b'NEEDLE nested committed'),
    ]:
        _, blob = run('rust', arguments)
        assert expected in blob and b'WRONG' not in blob, (arguments, blob)
        checks.append({'argv': arguments, 'route': 'Rust blob safety oracle', 'pass': True})
print(json.dumps({'checks': checks, 'passed': len(checks)}, indent=2))
