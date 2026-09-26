#!/usr/bin/env python3
"""Component-only C/Rust graph benchmark; includes process startup, excludes UI."""
import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import statistics
import subprocess
import time


def sha(data):
    return hashlib.sha256(data).hexdigest()


def run(binary, data, timeout):
    start = time.perf_counter()
    output = subprocess.run([str(binary)], input=data, capture_output=True,
                            timeout=timeout, check=True).stdout
    return time.perf_counter() - start, output


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--c-binary', required=True, type=Path)
    parser.add_argument('--rust-binary', required=True, type=Path)
    parser.add_argument('--repo', required=True, type=Path)
    parser.add_argument('--revision', required=True)
    parser.add_argument('--c-flags', required=True, help='Caller-supplied actual build flags')
    parser.add_argument('--rust-flags', default='cargo build --release; opt-level=3; lto=thin; codegen-units=1')
    parser.add_argument('--samples', type=int, default=16)
    parser.add_argument('--warmups', type=int, default=3)
    parser.add_argument('--timeout', type=float, default=30)
    parser.add_argument('--output', type=Path, default=Path(__file__).parent / 'evidence/benchmark.json')
    args = parser.parse_args()
    if args.samples < 16 or args.samples % 2 or args.warmups < 3:
        parser.error('An even sample count >=16 and at least 3 warmups are required')
    revision = subprocess.check_output(['git', '-C', str(args.repo), 'rev-parse', args.revision]).decode().strip()
    fixture_env = {key: value for key, value in os.environ.items() if not key.startswith('GIT_')}
    fixture_env.update(GIT_CONFIG_NOSYSTEM='1', GIT_CONFIG_GLOBAL=os.devnull, LC_ALL='C')
    history = subprocess.check_output(['git', '--no-replace-objects', '-C', str(args.repo),
                                      '-c', 'log.showSignature=false', '-c', 'log.mailmap=false',
                                      'log', '--pretty=raw', '--parents', '--no-decorate',
                                      '--no-color', '--no-notes', '--encoding=UTF-8', revision, '--'], env=fixture_env)
    linear = ''.join(f'commit {i}' + (f' {i+1}' if i < 9999 else '') + f'\n    linear {i}\n'
                     for i in range(10000)).encode()
    diamonds = []
    for i in range(1000):
        diamonds.append(f'commit m{i} a{i} b{i}\n    merge {i}\n'
                        f'commit a{i} m{i+1}\n    left {i}\n'
                        f'commit b{i} m{i+1}\n    right {i}\n')
    diamonds.append('commit m1000\n    root\n')
    workloads = {'tig_history': history, 'linear_10000': linear, 'diamonds_1000': ''.join(diamonds).encode()}
    binaries = {'c': args.c_binary.resolve(), 'rust': args.rust_binary.resolve()}
    hashes = {name: sha(path.read_bytes()) for name, path in binaries.items()}
    report = {'schema': 1, 'scope': 'graph helper component; process startup included; no UI performance claim',
              'revision': revision, 'platform': platform.platform(), 'machine': platform.machine(),
              'python': platform.python_version(), 'fixture_git_config': 'System/global config and inherited GIT_* disabled; signatures, mailmap, decorations, notes and color explicitly disabled; replace objects disabled', 'compilers': {
                  'c': subprocess.check_output(['cc', '--version']).decode().splitlines()[0],
                  'rust': subprocess.check_output(['rustc', '--version']).decode().strip()},
              'build_flags': {'c': args.c_flags, 'rust': args.rust_flags},
              'binaries': {name: {'path': str(path), 'sha256': hashes[name], 'bytes': path.stat().st_size}
                           for name, path in binaries.items()},
              'warmups_per_binary': args.warmups, 'samples_per_binary': args.samples,
              'order': 'alternating C/Rust then Rust/C; balanced with even sample count', 'workloads': {}}
    for name, data in workloads.items():
        reference = run(binaries['c'], data, args.timeout)[1]
        if run(binaries['rust'], data, args.timeout)[1] != reference:
            raise RuntimeError(f'Output mismatch before timing: {name}')
        for _ in range(args.warmups):
            for binary in binaries.values():
                if run(binary, data, args.timeout)[1] != reference:
                    raise RuntimeError(f'Output mismatch during warmup: {name}')
        samples = {'c': [], 'rust': []}
        for repeat in range(args.samples):
            for kind in (('c', 'rust') if repeat % 2 == 0 else ('rust', 'c')):
                elapsed, output = run(binaries[kind], data, args.timeout)
                if output != reference:
                    raise RuntimeError(f'Output mismatch during timing: {name}/{kind}')
                samples[kind].append(elapsed)
        summary = {kind: {'median_seconds': statistics.median(values),
                          'p95_seconds': sorted(values)[math.ceil(len(values) * .95) - 1],
                          'samples_seconds': values} for kind, values in samples.items()}
        report['workloads'][name] = {'input_bytes': len(data), 'input_sha256': sha(data),
                                     'stdout_bytes': len(reference), 'stdout_sha256': sha(reference),
                                     'byte_identical': True, 'timing': summary,
                                     'rust_over_c_median': summary['rust']['median_seconds'] / summary['c']['median_seconds']}
    if hashes != {name: sha(path.read_bytes()) for name, path in binaries.items()}:
        raise RuntimeError('A binary changed during measurement; rerun with stable binaries')
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps({name: value['rust_over_c_median'] for name, value in report['workloads'].items()}))
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
