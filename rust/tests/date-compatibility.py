#!/usr/bin/env python3
"""Run after cargo build --release. Subprocesses isolate TZ, locale and test time."""
import os
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]
BINARY = ROOT / 'target/release/tig'


def check(temporary, timestamp, setting, expected, args=('--pretty=raw',), **overrides):
    directory = Path(temporary)
    config = directory / 'tigrc'
    config.write_text('set main-view = date:default commit-title:yes,graph=no,refs=no\n' + setting)
    script = directory / 'steps'
    screen = directory / 'screen'
    script.write_text(f':save-display {screen}\n')
    env = {k: v for k, v in os.environ.items() if not k.startswith(('TIG', 'GIT_', 'XDG_'))}
    env.update(TIGRC_SYSTEM='', TIGRC_USER=str(config), TIG_SCRIPT=str(script),
               LC_ALL='C', TZ='UTC', COLUMNS='160', LINES='5', TEST_TIME_NOW='1441051553')
    env.update(overrides)
    raw = (f'commit {"a" * 40}\nauthor A <a@example.invalid> {timestamp}\n'
           f'committer A <a@example.invalid> {timestamp}\n\n    subject\n')
    result = subprocess.run([str(BINARY), *args], input=raw, text=True,
                            capture_output=True, cwd=directory, env=env, timeout=10)
    if expected is None:
        assert result.returncode != 0 and 'tig: ' in result.stderr, result
        assert 'panicked' not in result.stderr, result.stderr
    else:
        assert result.returncode == 0 and not result.stderr, result.stderr
        assert screen.read_text().splitlines()[0].rstrip() == expected, screen.read_text()


with tempfile.TemporaryDirectory(prefix='tig-date-') as temporary:
    custom = 'set main-view-date = custom\nset main-view-date-format = "%F %T %z %Z"\n'
    local = custom + 'set main-view-date-local = yes\n'
    cases = [
        ('0 +0000', '', '', {}),
        ('0 +0000', local, '', {'TZ': 'America/New_York'}),
        ('0 +0000', 'set main-view-date = relative\n', '', {}),
        ('-32400 +0900', custom, '', {}),
        ('0 +0900', '', '1970-01-01 09:00 +0900', {}),
        ('1710053999 +0000', local, '2024-03-10 01:59:59 -0500 EST', {'TZ': 'America/New_York'}),
        ('1710054000 +0000', local, '2024-03-10 03:00:00 -0400 EDT', {'TZ': 'America/New_York'}),
        ('1710054000 +0000', local, '2024-03-10 03:00:00 -0400 EDT', {'TZ': 'EST5EDT,M3.2.0,M11.1.0'}),
        ('1440961292 +0900', custom, '2015-08-31 04:01:32 +0900 +0900', {'TZ': 'America/New_York'}),
        ('-1 +0000', 'set main-view-date = relative\n', '0 second ago', {'TEST_TIME_NOW': '-1'}),
        ('1441051554 +0000', 'set main-view-date = relative\n', '1 second ahead', {}),
        ('1441051554 +0000', 'set main-view-date = relative-compact\n', '-1s', {}),
        ('1440961292 +0900', 'set main-view-date = relative\n', None, {'TEST_TIME_NOW': 'invalid'}),
        ('1440961292 +0900', 'set main-view-date = relative\n', None, {'TEST_TIME_NOW': str(2**63-1)}),
        ('1440961292 +0900', 'set main-view-date = custom\nset main-view-date-format = "%"\n', None, {}),
        ('1440961292 +0900', 'set main-view-date = custom\nset main-view-date-format = "%Q"\n', None, {}),
        ('1440961292 +2460', '', None, {}),
    ]
    # Exercise a non-English locale when installed; never silently count a skip as a pass.
    locales = subprocess.check_output(['locale', '-a'], text=True).splitlines()
    french = next((locale for locale in locales if locale.lower() in ('fr_fr.utf-8', 'fr_fr.utf8')), None)
    if french:
        cases.append(('1440961292 +0900',
                      'set main-view-date = custom\nset main-view-date-format = "%A %B"\n',
                      'lundi août', {'LC_ALL': french}))
    else:
        print('SKIP: fr_FR UTF-8 locale is not installed')
    for timestamp, setting, expected, overrides in cases:
        check(temporary, timestamp, setting, None if expected is None else expected + ' subject', **overrides)
    for boundary in ['--', '--end-of-options']:
        check(temporary, '1440961292 +0900', '', 'commit ' + 'a' * 40,
              args=(boundary, '--pretty=raw'))
    print(f'{len(cases) + 2} date compatibility checks passed')
