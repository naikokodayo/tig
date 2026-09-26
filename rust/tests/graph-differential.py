#!/usr/bin/env python3
"""Compare Rust graph output byte-for-byte with the original C test helper."""
import argparse
import hashlib
import json
from pathlib import Path
import random
import re
import subprocess
import time


def main():
    root = Path(__file__).resolve().parents[2]
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--c-binary', required=True, type=Path)
    parser.add_argument('--rust-binary', type=Path, default=root / 'target/debug/test-graph')
    parser.add_argument('--seeds', type=int, default=1000)
    parser.add_argument('--timeout', type=float, default=10)
    parser.add_argument('--output', type=Path, default=root / 'migration/evidence/graph-differential.json')
    args = parser.parse_args()
    fixtures = []
    for path in sorted((root / 'test/graph').iterdir()):
        if path.suffix == '.in':
            fixtures.append((path.name, path.read_bytes()))
        elif path.name.endswith('-test'):
            pattern = rb'test_graph[^\n]*<<[\x27\x22]?EOF[\x27\x22]?\n(.*?)\nEOF'
            for index, match in enumerate(re.finditer(pattern, path.read_bytes(), re.S)):
                fixtures.append((f'{path.name}:{index}', match[1] + b'\n'))
    originals = len(fixtures)
    for seed in range(args.seeds):
        rng = random.Random(seed)
        lines = []
        for commit in range(50):
            parents = rng.sample(range(commit + 1, 50), min(rng.randrange(5), 49 - commit))
            lines.append(f'commit {commit} ' + ' '.join(map(str, parents)) + f'\n    commit {commit}\n')
        fixtures.append((f'random:{seed}', ''.join(lines).encode()))
    binaries = [args.c_binary.resolve(), args.rust_binary.resolve()]
    binary_hashes = [hashlib.sha256(binary.read_bytes()).hexdigest() for binary in binaries]
    results = []
    totals = [0.0, 0.0]
    started = time.monotonic()
    for name, data in fixtures:
        for mode in ('utf8', 'ascii'):
            outputs = []
            for index, binary in enumerate(binaries):
                before = time.monotonic()
                process = subprocess.run([str(binary)] + (['--ascii'] if mode == 'ascii' else []),
                                         input=data, capture_output=True, timeout=args.timeout, check=True)
                totals[index] += time.monotonic() - before
                outputs.append(process.stdout)
            equal = outputs[0] == outputs[1]
            results.append({'fixture': name, 'mode': mode, 'equal': equal,
                            'input_sha256': hashlib.sha256(data).hexdigest(),
                            'c_output_sha256': hashlib.sha256(outputs[0]).hexdigest(),
                            'rust_output_sha256': hashlib.sha256(outputs[1]).hexdigest()})
    if binary_hashes != [hashlib.sha256(binary.read_bytes()).hexdigest() for binary in binaries]:
        raise RuntimeError('A binary changed during comparison; rerun with stable binaries')
    report = {
        'schema': 1,
        'binaries': {kind: {'path': str(binary), 'sha256': digest}
                     for kind, binary, digest in zip(('c', 'rust'), binaries, binary_hashes)},
        'upstream_fixture_count': originals, 'random_seed_range': [0, args.seeds],
        'random_commits_per_seed': 50, 'comparisons': len(results),
        'passed': sum(result['equal'] for result in results),
        'elapsed_seconds': time.monotonic() - started,
        'process_seconds': dict(zip(('c', 'rust'), totals)),
        'timeout_seconds_per_process': args.timeout,
        'limitations': ['GH490 gzip is a main-view input, not a test-graph fixture, and is excluded.',
                        'Compares ASCII/UTF8 output; C curses attributes and color IDs are not compared.',
                        'Process times include startup and are evidence, not a controlled benchmark.'],
        'results': results,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2) + '\n')
    print(f"{report['passed']}/{len(results)} byte-identical comparisons; evidence: {args.output}")
    return 0 if report['passed'] == len(results) else 1


if __name__ == '__main__':
    raise SystemExit(main())
