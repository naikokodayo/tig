#!/usr/bin/env python3
"""Run unchanged upstream scripts against C, then Rust, with fail-closed receipts.

Usage: python3 rust/tests/upstream-suite.py [test/graph/00-simple-test ...]
       python3 rust/tests/upstream-suite.py --self-test
Requires a POSIX controlling PTY, Python stdlib, Git, make, C compiler and Cargo.
Do not run concurrently with another upstream suite in this checkout (test/tmp).
"""
import argparse
from collections import Counter
import errno
import fcntl
import hashlib
import json
import math
import os
from pathlib import Path
import pty
import re
import selectors
import shutil
import signal
import struct
import subprocess
import sys
import tempfile
import termios
import time

ROOT = Path(__file__).resolve().parents[2]


def sha256(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def terminal(command, environment, seconds):
    """Bound the whole script, including setup, helpers and descendant processes."""
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 30, 80, 0, 0))

    def controlling_tty():
        os.setsid()
        fcntl.ioctl(0, termios.TIOCSCTTY, 0)

    output = bytearray()
    timed_out = False
    process = None
    started = time.monotonic()

    def kill_session():
        try:
            os.killpg(process.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        except PermissionError:
            # macOS may return EPERM for an already-dead process group.
            if process.poll() is None:
                raise

    try:
        process = subprocess.Popen(command, cwd=ROOT, env=environment, stdin=slave,
                                   stdout=slave, stderr=slave, preexec_fn=controlling_tty)
        os.close(slave)
        slave = None
        with selectors.DefaultSelector() as selector:
            selector.register(master, selectors.EVENT_READ)
            while selector.get_map() or process.poll() is None:
                if time.monotonic() - started >= seconds:
                    timed_out = True
                    kill_session()
                    break
                for key, _ in selector.select(0.05):
                    try:
                        data = os.read(key.fd, 65536)
                    except OSError as error:
                        if error.errno != errno.EIO:
                            raise
                        data = b''
                    if not data:
                        selector.unregister(key.fd)
                    output.extend(data)
                if process.poll() is not None:
                    # Timeout watchers may inherit the PTY after the script exits.
                    # Terminate that session too; never wait for escaped test work.
                    kill_session()
            code = process.wait(timeout=5)
    finally:
        if process is not None:
            kill_session()
            process.wait(timeout=5)
        os.close(master)
        if slave is not None:
            os.close(slave)
    return code, timed_out, output.decode(errors='replace')


def read(path):
    return path.read_text(errors='replace') if path.is_file() else ''


def collect(script, code, timed_out, transcript):
    directory = ROOT / 'test/tmp' / script.relative_to(ROOT / 'test')
    receipt = read(directory / '.test-result')
    markers = re.findall(r'^ *\[(OK|FAIL)\] (.*)$', receipt, re.M)
    skipped = read(directory / '.test-skipped')
    todos = {p.name: read(p) for p in sorted(directory.glob('.test-skipped-subtest-*'))}
    reasons = []
    if timed_out:
        reasons.append('script_timeout')
    if code != 0:
        reasons.append('script_exit_nonzero')
    if any(kind == 'FAIL' for kind, _ in markers):
        reasons.append('failed_checks')
    if not markers and not skipped:
        reasons.append('missing_receipt')
    assertions = []
    occurrences = Counter()
    for kind, message in markers:
        # Normalize only original assertion messages, not execution failures.
        match = re.fullmatch(r'(.+?) (?:assertion|does not exist|!= expected/.*|not found|should not exist)', message)
        if match:
            name = match[1]
            occurrences[name] += 1
            assertions.append({'id': f'{name}#{occurrences[name]}', 'status': kind})
    status = 'fail' if reasons else 'skip' if skipped else 'partial' if todos else 'pass'
    return {'script': str(script.relative_to(ROOT)), 'script_sha256': sha256(script),
            'exit_code': code, 'timed_out': timed_out, 'status': status, 'reasons': reasons,
            'checks': dict(Counter(kind for kind, _ in markers)), 'assertions': assertions,
            'receipt_path': str((directory / '.test-result').relative_to(ROOT)),
            'receipt': receipt, 'skip': skipped, 'skipped_cases': todos,
            'transcript': transcript}


def run_script(script, environment, seconds):
    directory = ROOT / 'test/tmp' / script.relative_to(ROOT / 'test')
    # libtest normally removes this, but a failure before sourcing it must not reuse a receipt.
    if directory.exists():
        shutil.rmtree(directory)
    return collect(script, *terminal([str(script.relative_to(ROOT))], environment, seconds))


def environment(paths):
    env = {k: v for k, v in os.environ.items()
           if not k.startswith(('GIT_', 'TIG_', 'TIGRC_', 'XDG_', 'CARGO_'))
           and k not in ('TEST_OPTS', 'MAKE_TEST_OPTS', 'MAKEFLAGS', 'MFLAGS')}
    env.update(PATH=os.pathsep.join(map(str, paths)) + os.pathsep + os.environ['PATH'],
               GIT_CONFIG_NOSYSTEM='1',
               GIT_CONFIG_COUNT='1', GIT_CONFIG_KEY_0='init.defaultBranch',
               GIT_CONFIG_VALUE_0='master', TEST_OPTS='', MAKE_TEST_OPTS='no-indent')
    return env


def routing(env, expected):
    resolved = {name: Path(shutil.which(name, path=env['PATH']) or '/missing').resolve()
                for name in expected}
    if resolved != expected or not all(p.is_file() and os.access(p, os.X_OK) for p in resolved.values()):
        raise RuntimeError(f'Executable routing mismatch: {resolved}; expected {expected}')
    return {name: {'path': str(path), 'sha256': sha256(path)} for name, path in resolved.items()}


def self_test():
    # Exercise the real, unchanged graph script and real libtest.sh, not a mock judge.
    script = ROOT / 'test/graph/00-simple-test'
    with tempfile.TemporaryDirectory(prefix='tig-harness-negative-') as temporary:
        directory = Path(temporary)
        helper = directory / 'test-graph'
        env = environment([directory, ROOT / 'test/tools'])
        findings = []
        code, timed_out, size = terminal(['/bin/sh', '-c', 'stty size </dev/tty'], env, 5)
        assert code == 0 and not timed_out and size.strip() == '30 80', size
        findings.append({'check': 'physical-pty-size', 'rows': 30, 'columns': 80})
        for name, body, timeout, reason in (
                ('nonzero', 'exit 23', 5, 'script_exit_nonzero'),
                ('wrong-output', "printf 'wrong output\\n'", 5, 'failed_checks'),
                ('timeout', 'exec sleep 30', 0.5, 'script_timeout')):
            helper.write_text('#!/bin/sh\n' + body + '\n')
            helper.chmod(0o755)
            result = run_script(script, env, timeout)
            assert result['status'] == 'fail' and reason in result['reasons'], result
            findings.append({'injection': name, 'helper_body': body, 'result': result})
        # An apparently good receipt cannot hide a later exit, timeout, or missing receipt.
        good = '  [OK] stdout assertion\n'
        fixture = ROOT / 'test/tmp/graph/00-simple-test/.test-result'
        fixture.parent.mkdir(parents=True, exist_ok=True)
        fixture.write_text(good)
        assert collect(script, 23, False, '')['status'] == 'fail'
        assert collect(script, 0, True, '')['status'] == 'fail'
        c_receipt = collect(script, 0, False, '')
        fixture.unlink()
        missing = collect(script, 0, False, '')
        assert missing['status'] == 'fail'
        assert compare(c_receipt, missing)['assertions'] == [
            {'id': 'stdout#1', 'c': 'OK', 'rust': 'NOT_REACHED'}]
        findings.append({'injection': 'good-receipt-then-exit-or-timeout; missing-receipt-exit-zero',
                         'status': 'all rejected'})
        # A C helper on PATH cannot satisfy an explicit Rust route.
        try:
            routing(env, {'test-graph': ROOT / 'target/release/test-graph'})
        except RuntimeError:
            findings.append({'injection': 'foreign-helper', 'status': 'rejected'})
        else:
            raise AssertionError('foreign helper accepted')
        stale = directory / 'stale.json'
        stale.write_text('{"parity_gate": "PASS_SELECTED_SCRIPTS"}')
        (directory / 'git').symlink_to(shutil.which('git'))
        crashed = subprocess.run([sys.executable, __file__, str(script), '--output', str(stale)],
                                 env={**env, 'PATH': str(directory)}, capture_output=True, text=True)
        assert crashed.returncode != 0 and 'FileNotFoundError' in crashed.stderr
        assert not stale.exists(), 'runner crash retained a previous passing report'
        findings.append({'injection': 'missing-make-with-stale-pass-report', 'status': 'rejected and stale report removed'})
    dry = subprocess.check_output(['make', '-Bn', 'RUST_ONLY=1', str(script.relative_to(ROOT))],
                                  cwd=ROOT, text=True)
    assert 'cargo build' in dry and 'test/tools/test-graph.o' not in dry and 'src/tig.o' not in dry
    print(json.dumps(findings, indent=2))


def compare(c, rust):
    c_asserts = {a['id']: a['status'] for a in c['assertions']}
    r_asserts = {a['id']: a['status'] for a in rust['assertions']}
    return {'script': c['script'], 'c_status': c['status'], 'rust_status': rust['status'],
            'assertions': [{'id': key, 'c': c_asserts.get(key, 'NOT_REACHED'),
                            'rust': r_asserts.get(key, 'NOT_REACHED')}
                           for key in dict.fromkeys([*c_asserts, *r_asserts])]}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('scripts', nargs='*', type=Path)
    parser.add_argument('--output', type=Path, default=ROOT / 'migration/evidence/upstream-rust-only.json')
    parser.add_argument('--script-timeout', type=float, default=120)
    parser.add_argument('--self-test', action='store_true')
    args = parser.parse_args()
    if args.self_test:
        self_test()
        return 0
    if not math.isfinite(args.script_timeout) or args.script_timeout <= 0:
        parser.error('--script-timeout must be positive')
    # A crashed build/runner must not leave a previous passing report in place.
    args.output.unlink(missing_ok=True)
    tracked = subprocess.check_output(['git', 'ls-files', '-z', 'test/*-test'], cwd=ROOT).decode().split('\0')
    originals = {ROOT / name for name in tracked if name}
    scripts = sorted({p.resolve() for p in args.scripts}) if args.scripts else sorted(originals)
    if not scripts:
        parser.error('No original test scripts selected')
    for script in scripts:
        if script not in originals or not script.is_file():
            parser.error(f'Not an original test script: {script}')
    report = {'schema': 1, 'source_commit': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
              'scope': 'Original scripts and assertions; C baseline followed by Rust application AND graph helper',
              'harness_sha256': {name: sha256(ROOT / name) for name in ('Makefile', 'test/tools/libtest.sh', 'rust/tests/upstream-suite.py')},
              'rust_complete': False, 'parity_gate': 'BLOCKED', 'script_timeout_seconds': args.script_timeout,
              'environment': {'platform': os.uname().sysname + ' ' + os.uname().machine,
                              'git': subprocess.check_output(['git', '--version'], text=True).strip(),
                              'init.defaultBranch': 'master', 'TEST_OPTS': '', 'pty_rows': 30, 'pty_columns': 80}, 'runs': {}}
    for mode in ('c', 'rust'):
        build = ['make', 'src/tig', 'test/tools/test-graph'] if mode == 'c' else ['make', 'rust-test-binaries']
        dirs = [ROOT / 'src', ROOT / 'test/tools'] if mode == 'c' else [ROOT / 'target/release', ROOT / 'test/tools']
        env = environment(dirs)
        expected = {'tig': dirs[0] / 'tig',
                    'test-graph': (ROOT / 'test/tools' if mode == 'c' else dirs[0]) / 'test-graph'}
        built = subprocess.run(build, cwd=ROOT, env=env, capture_output=True, text=True)
        run = {'build_command': build, 'build_exit_code': built.returncode,
               'build_log': built.stdout + built.stderr, 'results': []}
        report['runs'][mode] = run
        if built.returncode:
            break
        run['binaries'] = routing(env, expected)
        for script in scripts:
            result = run_script(script, env, args.script_timeout)
            run['results'].append(result)
            print(f"{mode}: {result['status']:7} {result['script']}", flush=True)
        run['binaries_unchanged'] = routing(env, expected) == run['binaries']
        run['summary'] = dict(Counter(r['status'] for r in run['results']))
    if all(report['runs'].get(mode, {}).get('binaries_unchanged') for mode in ('c', 'rust')):
        report['mapping'] = [compare(c, r) for c, r in zip(report['runs']['c']['results'], report['runs']['rust']['results'])]
        if all(r['status'] == 'pass' for run in report['runs'].values() for r in run['results']):
            if all(a['c'] == a['rust'] == 'OK' for row in report['mapping'] for a in row['assertions']):
                report['parity_gate'] = 'PASS_SELECTED_SCRIPTS'
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2, ensure_ascii=False) + '\n')
    print(f"{report['parity_gate']}: {args.output}")
    return int(report['parity_gate'] == 'BLOCKED')


if __name__ == '__main__':
    raise SystemExit(main())
