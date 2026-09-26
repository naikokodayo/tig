#!/usr/bin/env python3
"""Build temporary v1 oracles and compare glyphs, colors, and merge flags.

Requires an already-built original C tree; no sources or build objects are modified.
Example: python3 rust/tests/graph-v1-differential.py --c-source ../../work/tig-build --c-build ../../work/tig-build
"""
import argparse
import hashlib
import json
from pathlib import Path
import shlex
import subprocess
import sys
import tempfile


META = r'''
fn render_meta(canvas: &graph::Canvas, ascii: bool) -> String {
    let mut text = String::new();
    for (i, symbol) in canvas.symbols.iter().enumerate() {
        let chars = if ascii { symbol.ascii() } else { symbol.utf8() };
        text.push_str(&format!("{}:{}", symbol.color_id(), if i == 0 { &chars[1..] } else { chars }));
    }
    format!("{} {}", text, usize::from(canvas.is_merge()))
}
'''


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def replace_once(source, old, new):
    if source.count(old) != 1:
        raise RuntimeError(f'Upstream helper changed: expected one {old!r}')
    return source.replace(old, new)


def main():
    root = Path(__file__).resolve().parents[2]
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--c-source', required=True, type=Path)
    parser.add_argument('--c-build', required=True, type=Path)
    parser.add_argument('--seeds', type=int, default=1000)
    parser.add_argument('--timeout', type=float, default=10)
    parser.add_argument('--output', type=Path, default=root / 'migration/evidence/graph-v1-differential.json')
    args = parser.parse_args()
    c_source, c_build = args.c_source.resolve(), args.c_build.resolve()
    commands = []

    def run(command, **kwargs):
        commands.append([str(arg) for arg in command])
        return subprocess.run(command, check=True, capture_output=True, **kwargs)

    # Dry-run the existing build to obtain its actual compiler/linker configuration.
    dry = run(['make', '-n', '-B', '-C', str(c_build), 'DIST_VERSION=graph-v1-oracle',
               'test/tools/test-graph'], timeout=60).stdout.decode()
    link = next(line.rsplit(';', 1)[-1] for line in reversed(dry.splitlines())
                if ' -o test/tools/test-graph' in line)
    link = shlex.split(link)
    sources = [c_source / 'test/tools/test-graph.c', c_source / 'src/graph-v1.c',
               root / 'rust/graph_v1.rs', root / 'rust/bin/test-graph.rs',
               root / 'rust/tests/graph-differential.py', Path(__file__).resolve()]
    objects = [c_build / arg for arg in link if arg.endswith('.o') and arg != 'test/tools/test-graph.o']
    tracked = sources + objects
    hashes = {str(path): digest(path) for path in tracked}
    c_text = replace_once(sources[0].read_text(), 'GRAPH_DISPLAY_V2', 'GRAPH_DISPLAY_V1')
    rust_text = replace_once(sources[3].read_text(), 'use tig_rs::graph;',
                             '#[path = ' + json.dumps(str(sources[2])) + ']\nmod graph;')
    receipt = {'schema': 1, 'scope': 'v1 ASCII/UTF8 glyphs, per-symbol color IDs, canvas merge flag',
               'source_and_object_sha256': hashes, 'compiler_versions': {
                   'c': run([link[0], '--version']).stdout.decode().splitlines()[0],
                   'rust': run(['rustc', '--version']).stdout.decode().strip()}, 'runs': {}}
    with tempfile.TemporaryDirectory(prefix='tig-graph-v1-') as temporary:
        temp = Path(temporary)
        for mode in ('glyphs', 'metadata'):
            c = temp / (mode + '.c')
            rust = temp / (mode + '.rs')
            c.write_text(c_text if mode == 'glyphs' else replace_once(replace_once(c_text,
                'printf("%s", chars + !!first);', 'printf("%d:%s", color_id, chars + !!first);'),
                'printf(" %s\\n", title);', 'printf(" %d %s\\n", graph->is_merge(&commit->canvas), title);'))
            rust.write_text(rust_text if mode == 'glyphs' else
                            rust_text.replace('rendered.render(ascii)', 'render_meta(&rendered, ascii)') + META)
            c_binary, rust_binary = temp / ('c-' + mode), temp / ('rust-' + mode)
            command = []
            for arg in link:
                if arg == 'test/tools/test-graph.o':
                    command.append(str(c))
                elif arg == 'test/tools/test-graph':
                    command.append(str(c_binary))
                elif arg.endswith('.o'):
                    command.append(str(c_build / arg))
                else:
                    command.append(arg)
            command[1:1] = ['-I' + str(c_build), '-I' + str(c_source / 'include')]
            run(command, cwd=c_build, timeout=60)
            run(['rustc', '--edition=2021', str(rust), '-o', str(rust_binary)], timeout=60)
            report_path = temp / (mode + '.json')
            run([sys.executable, str(sources[4]), '--c-binary', str(c_binary), '--rust-binary', str(rust_binary),
                 '--seeds', str(args.seeds), '--timeout', str(args.timeout), '--output', str(report_path)])
            result = json.loads(report_path.read_text())
            result['adapted_helper_sha256'] = {'c': digest(c), 'rust': digest(rust)}
            result['limitations'] = ['GH490 main-view fixture excluded; curses chtype output not compared.']
            receipt['runs'][mode] = result
    if hashes != {str(path): digest(path) for path in tracked}:
        raise RuntimeError('Sources or original C objects changed during comparison; rerun')
    receipt['commands'] = commands
    receipt['comparisons'] = sum(run['comparisons'] for run in receipt['runs'].values())
    receipt['passed'] = sum(run['passed'] for run in receipt['runs'].values())
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(receipt, indent=2) + '\n')
    print(f"{receipt['passed']}/{receipt['comparisons']} v1 comparisons passed; {args.output}")
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
