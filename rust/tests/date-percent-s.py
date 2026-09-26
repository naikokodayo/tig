#!/usr/bin/env python3
"""Host C/Rust %s differential through real controlling PTYs (Linux/macOS).

Build src/tig and target/release/tig first. No original test is modified.
"""
import argparse
from datetime import datetime, timezone
import importlib.util
import json
import os
from pathlib import Path
import platform
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location('upstream', ROOT / 'rust/tests/upstream-suite.py')
upstream = importlib.util.module_from_spec(spec)
spec.loader.exec_module(upstream)


def seconds(date):
    return int(datetime.fromisoformat(date).replace(tzinfo=timezone.utc).timestamp())


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, default=ROOT / 'target/date-percent-s.json')
    args = parser.parse_args()
    args.output.unlink(missing_ok=True)
    binaries = [ROOT / 'src/tig', ROOT / 'target/release/tig']
    report = {'source_commit': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
              'test_sha256': upstream.sha256(Path(__file__)),
              'platform': platform.platform(), 'scope': 'Host C/Rust custom %s PTY differential',
              'binary_sha256': {str(p.relative_to(ROOT)): upstream.sha256(p) for p in binaries},
              'cases': []}
    cases = []
    for zone in ['UTC', 'America/New_York', 'Asia/Kolkata', 'Australia/Lord_Howe',
                 'Europe/Dublin', 'Africa/Casablanca', 'Pacific/Apia',
                 'EST5EDT,M3.2.0,M11.1.0', 'XXX-5:30', '', None]:
        for instant in ['2024-01-01', '2024-07-01']:
            for offset in ['+0000', '+0900', '-0700']:
                for local in [False, True]:
                    cases.append((zone, seconds(instant), offset, local, '%F %T s=%s z=%z Z=%Z'))
    # Wall times in and around the spring gap and autumn fold, not UTC instants.
    for instant in ['2024-03-10T01:59:59', '2024-03-10T02:00:00',
                    '2024-03-10T02:30:00', '2024-03-10T03:00:00',
                    '2024-11-03T00:59:59', '2024-11-03T01:00:00',
                    '2024-11-03T01:30:00', '2024-11-03T02:00:00']:
        cases.append(('America/New_York', seconds(instant), '+0000', False, '%F %T s=%s z=%z'))
    for value in [-62135596800, -2208988800, -1, 0, 1, 2147483647, 2147483648, 253402300799]:
        cases.append(('America/New_York', value, '+0000', False, '%s'))
    for fmt in ['%%s', '%%%s', '%s/%s', '%%s %s %%', '%A %B %s']:
        cases.append(('America/New_York', seconds('2024-07-01'), '+0900', False, fmt))
    cases.append(('UTC', -32400, '+0900', False, '%s'))
    cases.append(('UTC', 0, '+0900', False, '%s'))
    # Named file and absolute-file TZ forms use the same host database as C.
    cases.append((':America/New_York', seconds('2024-07-01'), '+0000', False, '%s'))
    if Path('/usr/share/zoneinfo/America/New_York').is_file():
        cases.append((':/usr/share/zoneinfo/America/New_York', seconds('2024-07-01'), '+0000', False, '%s'))
    with tempfile.TemporaryDirectory(prefix='tig-percent-s-') as temporary:
        directory = Path(temporary)
        config, script, raw, screen = [directory / p for p in ['config', 'script', 'raw', 'screen']]
        script.write_text(f':save-display {screen}\n:quit\n')
        env = {k: v for k, v in os.environ.items() if not k.startswith(('TIG', 'GIT_', 'XDG_', 'TEST_'))}
        env.update(TIGRC_SYSTEM='', TIGRC_USER=str(config), TIG_SCRIPT=str(script),
                   TERM='xterm', LC_ALL='C', LINES='5', COLUMNS='160')
        for zone, value, offset, local, fmt in cases:
            if zone is None:
                env.pop('TZ', None)
            else:
                env['TZ'] = zone
            config.write_text('set main-view = date:default commit-title:yes,graph=no,refs=no\n'
                              'set main-view-date = custom\n'
                              f'set main-view-date-format = "{fmt}"\n'
                              f'set main-view-date-local = {"yes" if local else "no"}\n')
            raw.write_text(f'commit {"a" * 40}\nauthor A <a@example.invalid> {value} {offset}\n'
                           f'committer A <a@example.invalid> {value} {offset}\n\n    subject\n')
            rows = []
            for binary in binaries:
                screen.unlink(missing_ok=True)
                code, timeout, transcript = upstream.terminal(
                    ['sh', '-c', 'exec "$1" --pretty=raw < "$2"', 'sh', str(binary), str(raw)], env, 10)
                rows.append({'exit_code': code, 'timeout': timeout,
                             'row': screen.read_text().splitlines()[0].rstrip() if screen.exists() else None,
                             'error': transcript[-500:] if code else ''})
            passed = all(r['exit_code'] == 0 and not r['timeout'] and r['row'] is not None for r in rows) and rows[0]['row'] == rows[1]['row']
            report['cases'].append({'TZ': zone, 'raw': f'{value} {offset}', 'local': local,
                                    'format': fmt, 'C': rows[0], 'Rust': rows[1], 'passed': passed})
    report['binaries_unchanged'] = all(upstream.sha256(p) == report['binary_sha256'][str(p.relative_to(ROOT))] for p in binaries)
    report['passed'] = report['binaries_unchanged'] and all(case['passed'] for case in report['cases'])
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2) + '\n')
    failures = [case for case in report['cases'] if not case['passed']]
    print(f'{len(cases) - len(failures)}/{len(cases)} host C/Rust %s checks passed; {args.output}')
    if failures:
        print(json.dumps(failures[:3], indent=2))
    return 0 if report['passed'] else 1


if __name__ == '__main__':
    raise SystemExit(main())
