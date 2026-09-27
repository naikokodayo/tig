#!/usr/bin/env python3
"""Unix native CLI filename regression; real Git and C/Rust controlling PTYs.

No receipt files are generated. Requires built src/tig and target/release/tig.
An optional --baseline binary proves the old argv rejection before the fix.
"""
import argparse
import errno
import hashlib
import importlib.util
import os
from pathlib import Path
import re
import shlex
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location('upstream', ROOT / 'rust/tests/upstream-suite.py')
upstream = importlib.util.module_from_spec(spec)
spec.loader.exec_module(upstream)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--baseline', type=Path)
    options = parser.parse_args()
    if os.name != 'posix':
        raise SystemExit('This byte-filename/controlling-PTY check requires Unix')
    passed = 0
    with tempfile.TemporaryDirectory(prefix='tig-cli-paths-') as temporary:
        home = Path(temporary)
        repo = home / 'repo'
        repo.mkdir()
        env = upstream.environment([])
        env.update(HOME=temporary, GIT_CONFIG_GLOBAL='/dev/null', TIGRC_SYSTEM='',
                   TIGRC_USER=str(home / 'tigrc'), TIG_SCRIPT=str(home / 'steps'),
                   TERM='xterm-256color', LINES='30', COLUMNS='200', LC_ALL='C',
                   GIT_AUTHOR_NAME='Paths', GIT_AUTHOR_EMAIL='paths@example.invalid',
                   GIT_COMMITTER_NAME='Paths', GIT_COMMITTER_EMAIL='paths@example.invalid')
        (home / 'tigrc').write_text('set line-graphics = ascii\n'
                                  'set blame-view = line-number:yes,interval=1 text\n')

        def git(*args):
            return subprocess.check_output(['git', '-C', str(repo), *args], env=env)

        def put(name, text):
            with open(os.fsencode(repo) + b'/' + name, 'wb') as file:
                file.write(text.encode())

        old, new = b'old-\xfe name', b'new-\xff name'
        lossy = new.decode('utf-8', 'replace').encode()
        directory = b'sub-\xfd dir'
        content = ''.join(f'TARGET line {i:02d}\n' for i in range(1, 41))
        git('init', '-q')
        try:
            put(old, content)
        except OSError as error:
            if error.errno == errno.EILSEQ:
                raise SystemExit('UNSUPPORTED: this filesystem rejects non-UTF-8 names; run on Linux') from error
            raise
        put(lossy, 'DECOY replacement character filename\n')
        git('add', '--all')
        git('commit', '-qm', 'original')
        original = git('rev-parse', 'HEAD').decode().strip()
        git('mv', '--', old, new)
        put(new, content.replace('TARGET line 20', 'INSERTED target\nTARGET line 20'))
        os.mkdir(os.fsencode(repo) + b'/' + directory)
        put(directory + b'/file', content)
        put(b'file', 'DECOY root file\n')
        special = [b'--output=sentinel-\xfc', b':(glob)*-\xfb', b'tab\t-\xfa']
        for index, name in enumerate(special):
            put(name, content.replace('TARGET', f'SPECIAL-{index}'))
        git('add', '--all')
        git('commit', '-qm', 'rename and insert')
        tracked = git('ls-files', '-z').split(b'\0')[:-1]

        def digest():
            files = [os.fsencode(repo) + b'/' + name for name in tracked]
            files.append(os.fsencode(repo / '.git/index'))
            return [(name, hashlib.sha256(Path(os.fsdecode(name)).read_bytes()).hexdigest())
                    for name in files]

        before = digest()

        def screen(binary, args, steps='', dirs=None, extra=None, alias=False, error=None):
            target = home / 'screen'
            target.unlink(missing_ok=True)
            (home / 'steps').write_text(steps + f'\n:save-display {target}\n:quit\n')
            routed = dict(env, **(extra or {}))
            if alias:
                command = ['git', '-C', os.fsdecode(os.fsencode(repo) + b'/' + directory),
                           '-c', 'alias.tig=!' + shlex.quote(str(binary)), 'tig', *args]
            else:
                command = [str(binary), *(dirs or ['-C', str(repo)]), *args]
            code, timeout, transcript = upstream.terminal(command, routed, 15)
            assert not timeout, (command, transcript)
            if error:
                assert code != 0 and not target.exists() and error in transcript, (command, code, transcript)
                return transcript
            assert code == 0 and target.exists(), (command, code, transcript)
            text = target.read_text(errors='replace')
            assert 'DECOY' not in text, text
            return text

        c, rust = ROOT / 'src/tig', ROOT / 'target/release/tig'
        if options.baseline:
            transcript = screen(options.baseline.resolve(), ['blame', '--', new], error='Non-UTF-8 CLI arguments')
            print('BASELINE rejected byte filename: ' + re.sub(r'\x1b\[[0-9;?]*[a-zA-Z]', '', transcript).strip())

        def pair(name, args, steps='', expected='line 21 of 41', needle=' 21| TARGET line 20', **kwargs):
            nonlocal passed
            texts = [screen(binary, args, steps, **kwargs) for binary in (c, rust)]
            for text in texts:
                assert expected in text and needle in text, (name, text)
            # Filename rendering differs for invalid bytes; compare every actual row.
            rows = [re.findall(r'^\s*\d+\|.*$', text, re.M) for text in texts]
            assert rows[0] and rows[0] == rows[1], (name, texts)
            passed += 1
            print('PASS paired: ' + name, flush=True)
            return texts

        pair('explicit byte filename and +line', ['blame', '+21', '--', new])
        pair('implicit final byte filename', ['blame', new, '+21'])
        pair('explicit revision', ['blame', '+21', 'HEAD', '--', new])
        pair('refresh preserves path and line', ['blame', '+21', '--', new], ':refresh')
        # Original C passes Git's quoted historical filename back literally.
        # Retain the default-config failure; do not count it as navigation parity.
        failure = screen(c, ['blame', '+21', '--', new], ':view-blame', error='No blame exist for')
        print('C LIMITATION (core.quotePath=true): tig:' + failure.rsplit('tig:', 1)[-1].strip())
        for steps, position, row in [
            (':view-blame', 'line 20 of 40', ' 20| TARGET line 20'),
            (':view-blame\n:back', 'line 21 of 41', ' 21| TARGET line 20'),
            (':parent', 'line 19 of 40', ' 19| TARGET line 19'),
        ]:
            line = '+20' if steps == ':parent' else '+21'
            text = screen(rust, ['blame', line, '--', new], steps)
            assert position in text and row in text, text
            if steps != ':view-blame\n:back':
                assert original in text, text
            passed += 1
        print('PASS 3 Rust default-config native historical navigation checks')
        # Git emits raw high bytes with this setting, so C can follow the path.
        git('config', 'core.quotePath', 'false')
        texts = pair('historical byte filename and original line', ['blame', '+21', '--', new],
                     ':view-blame', expected='line 20 of 40', needle=' 20| TARGET line 20')
        assert all(original in text for text in texts), texts
        pair('back restores byte filename and line', ['blame', '+21', '--', new], ':view-blame\n:back')
        pair('parent maps inserted line to old byte filename', ['blame', '+20', '--', new],
             ':parent', expected='line 19 of 40', needle=' 19| TARGET line 19')
        for index, name in enumerate(special):
            pair('literal filename ' + repr(name), ['blame', '+20', '--', name],
                 expected='line 20 of 40', needle=f' 20| SPECIAL-{index} line 20')
        for name, kwargs in [
            ('native -C', dict(dirs=['-C', os.fsdecode(os.fsencode(repo) + b'/' + directory)])),
            ('sequential -C', dict(dirs=['-C', str(repo), '-C', directory])),
            ('native GIT_PREFIX', dict(extra={'GIT_PREFIX': os.fsdecode(directory + b'/')})),
            ('real Git shell alias prefix', dict(alias=True)),
        ]:
            pair(name, ['blame', '+20', '--', 'file'], expected='line 20 of 40',
                 needle=' 20| TARGET line 20', **kwargs)
        for args, error in [
            (['blame', '--', new, lossy], 'Blame requires exactly one file'),
            (['blame', '--', b''], 'Blame requires exactly one file'),
            (['blame', b'HEAD-\xff', '--', new], 'Non-UTF-8 blame options/revisions'),
            (['blame', '-L', b'\xff', '--', new], 'Non-UTF-8 blame option value'),
            (['blame', b'--output=\xff', '--', new], 'Non-UTF-8 blame options/revisions'),
            (['blame', '--output=sentinel', '--', new], 'Unsupported blame option'),
            (['blame', '--contents=file', '--', new], 'Unsupported blame option'),
            (['blame', '--textconv', '--', new], 'Unsupported blame option'),
            (['blame', '--', b'../escape-\xff'], "without '..'"),
            (['blame', '--', b'new-\xfd name'], 'no such path'),
            (['status', '--', new], 'Non-UTF-8 CLI arguments'),
            (['show', '--', new], 'Non-UTF-8 CLI arguments'),
            (['grep', 'TARGET', '--', new], 'Non-UTF-8 CLI arguments'),
        ]:
            screen(rust, args, error=error)
            passed += 1
        assert not (repo / 'sentinel').exists()
        assert before == digest(), 'blame changed tracked bytes or index'
        passed += 1
        print('PASS 13 rejected combinations; exact tracked-file and index hashes unchanged')
    print(f'PASS {passed} checks (14 paired PTY workflows, 3 Rust default navigation, 13 Rust rejections, 1 read-only check; C default navigation limitation separate)')


if __name__ == '__main__':
    main()
