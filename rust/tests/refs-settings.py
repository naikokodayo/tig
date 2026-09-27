#!/usr/bin/env python3
"""Focused refs settings regression through real C/Rust terminals (stdlib only)."""
import os
from pathlib import Path
import runpy
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]
terminal = runpy.run_path(str(ROOT / 'rust/tests/upstream-suite.py'))['terminal']
with tempfile.TemporaryDirectory(prefix='tig-refs-settings-') as temporary:
    directory = Path(temporary)
    terminal.__globals__['ROOT'] = directory
    env = {k: v for k, v in os.environ.items() if not k.startswith(('TIG', 'GIT_', 'XDG_'))}
    env.update(HOME=temporary, GIT_CONFIG_NOSYSTEM='1', GIT_CONFIG_GLOBAL='/dev/null',
               TERM='xterm', LC_ALL='en_US.UTF-8', TZ='UTC', COLUMNS='120', LINES='12',
               TIGRC_SYSTEM='', TIGRC_USER=str(directory / 'tigrc'),
               TIG_SCRIPT=str(directory / 'steps'))
    def git(*args):
        return subprocess.check_output(['git', *args], cwd=directory, env=env, text=True).strip()
    git('init', '-q', '-b', 'main')
    for branch, name, author_date, commit_date in [
        ('alpha', 'Zed', '2020-01-03T00:00:00+00:00', '2020-01-01T01:00:00+01:00'),
        ('beta', 'amy', '2020-01-01T00:00:00+00:00', '2020-01-03T00:00:00+00:00'),
        ('gamma', 'Zed', '2020-01-02T00:00:00+00:00', '2020-01-02T00:00:00+00:00'),
    ]:
        env.update(GIT_AUTHOR_NAME=name, GIT_AUTHOR_EMAIL=name+'@old.test',
                   GIT_COMMITTER_NAME=name, GIT_COMMITTER_EMAIL=name+'@old.test',
                   GIT_AUTHOR_DATE=author_date, GIT_COMMITTER_DATE=commit_date)
        (directory / 'file').write_text(branch+'\n')
        git('add', 'file')
        git('commit', '-qm', 'same subject')
        git('branch', branch)
    (directory / '.mailmap').write_text('Aaron <new@test> Zed <Zed@old.test>\n')
    for mailmap, date_options in [('no', ''), ('yes', ''), ('no', ',use-author=yes'),
                                  ('yes', ',use-author=yes'), ('yes', ',local=yes')]:
        (directory / 'tigrc').write_text(
            f'set mailmap = {mailmap}\nset refs-view = ref date:default{date_options} '
            'author:full,width=10 committer:email,width=15 commit-title\n')
        # Cover initial metadata, each available sort field, reverse and live mailmap refresh.
        actions = [':save-display initial', ':toggle sort-field', ':save-display date',
                   ':toggle sort-order', ':save-display reverse', ':toggle sort-order',
                   ':toggle sort-field', ':save-display author', ':toggle sort-field',
                   ':save-display committer', ':toggle sort-field', ':save-display title',
                   ':toggle mailmap', ':save-display toggled', ':quit']
        (directory / 'steps').write_text('\n'.join(actions)+'\n')
        captures = {}
        for label, binary in [('c', ROOT / 'src/tig'), ('rust', ROOT / 'target/release/tig')]:
            for name in ('initial', 'date', 'reverse', 'author', 'committer', 'title', 'toggled'):
                (directory / name).unlink(missing_ok=True)
            code, timeout, transcript = terminal([str(binary), 'refs'], env, 20)
            assert code == 0 and not timeout, (label, code, transcript)
            captures[label] = {}
            for name in ('initial', 'date', 'reverse', 'author', 'committer', 'title', 'toggled'):
                # Compare actual reference rows: the synthetic All row and status selection
                # are navigation policy, independent of these metadata/settings checks.
                captures[label][name] = [line.rstrip() for line in (directory / name).read_text().splitlines()
                                         if line.split() and line.split()[0] in ('main', 'alpha', 'beta', 'gamma')]
                assert len(captures[label][name]) == 4, (label, name, captures[label][name])
        for name in captures['c']:
            expected, actual = captures['c'][name], captures['rust'][name]
            if name == 'toggled':
                expected, actual = sorted(expected), sorted(actual)
            assert expected == actual, (mailmap, date_options, name, expected, actual)
        print(f'PASS mailmap={mailmap} date={date_options or "committer"}: 7 paired captures')
print('35 paired refs metadata/sort captures passed')
