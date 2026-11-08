"""Assert-based engine smoke checks with a native fixture runner, no model calls or windows.
Build hex-meetings and examples/daily_check_verify first. All writes stay in artifacts.
"""
import datetime
import json
import os
from pathlib import Path
import shutil
import subprocess

ROOT = Path(__file__).resolve().parents[1]
EXE = ROOT / 'target/x86_64-pc-windows-gnu/debug/hex-meetings.exe'
FIXTURE = ROOT / 'target/x86_64-pc-windows-gnu/debug/examples/daily_check_verify.exe'


def main():
    base = ROOT / 'artifacts/daily-check-verification' / datetime.datetime.now().strftime('%Y%m%d%H%M%S%f')
    appdata = base / 'daily-check-selfcheck'
    state = appdata / 'hex-meetings'
    meeting = appdata / 'hex/meetings/123-456'
    meeting.mkdir(parents=True)
    state.mkdir(parents=True)
    (base / 'bin').mkdir()
    (base / 'fail-git').mkdir()
    shutil.copy2(FIXTURE, base / 'bin/claude.exe')
    shutil.copy2(FIXTURE, base / 'fail-git/git.exe')
    (meeting / 'english-summary.json').write_text(json.dumps({'title': 'Fixture', 'todo': [{'task': 'Check shipment'}]}))
    (state / 'runners.json').write_text(json.dumps([{'name': 'Daily check (Opus)', 'program': str(base / 'bin/claude.exe'), 'model': 'fixture', 'args': ['-p'], 'stdin': True, 'tools': True}]))
    response = base / 'response.json'
    env = dict(os.environ, APPDATA=str(appdata), LOCALAPPDATA=str(base), HEX_DAILY_CHECK_FIXTURE=str(response))
    date = datetime.date.today().isoformat()
    report_path = state / f'checks/{date}.json'
    memory_path = state / 'memory/company.md'
    memory = '# Company\n\n' + '\n\n'.join(f'- Sourced fixture fact {n}. [[123-456]]' for n in range(40))
    valid = {'findings': [], 'story': 'Fixture [[123-456]]', 'memory_update': memory, 'summary': 'One\nTwo\nThree'}

    def run(payload, expected=0, overrides=None, extra=()):
        response.write_text(payload if isinstance(payload, str) else json.dumps(payload))
        result = subprocess.run([str(EXE), '--daily-check', '--force', *extra], env=dict(env, **(overrides or {})), capture_output=True, creationflags=subprocess.CREATE_NO_WINDOW, timeout=150)
        assert result.returncode == expected, (result.returncode, result.stderr.decode(errors='replace'))
        return json.loads(report_path.read_bytes()) if expected == 0 else None

    report = run(valid)
    assert report['memory_updated'] and memory_path.read_text() == memory
    second = dict(valid, memory_update=memory + '\n\n- New sourced fact. [[123-456]]')
    run(second)
    assert (state / f'memory/history/{date}.md').read_text() == memory
    current = memory_path.read_text()
    report = run(dict(valid, memory_update='# Company\n\n- Small update. [[123-456]]'))
    assert not report['memory_updated'] and report['findings'][-1]['title'] == 'Company memory update rejected'
    assert memory_path.read_text() == current
    prior_report = report_path.read_bytes()
    run('not JSON', 1)
    run(dict(valid, findings=[{'kind': 'other', 'severity': 'low', 'title': 'Bad source', 'explanation': 'Fixture', 'evidence': ['missing.md']}]), 1)
    assert report_path.read_bytes() == prior_report and memory_path.read_text() == current
    report = run(dict(valid, findings=[{'kind': 'other', 'severity': 'low', 'title': 'Bad target', 'explanation': 'Fixture', 'evidence': ['[[123-456]]'], 'action': {'kind': 'set_owner', 'meeting_id': '123-456', 'todo_index': 99, 'expected_task': 'Check shipment', 'value': 'Owner'}}]))
    assert len(report['findings']) == 1 and report['findings'][0].get('action') is None
    report = run(second, overrides={'PATH': str(base / 'fail-git') + os.pathsep + env['PATH']})
    assert report['kb']['stale'] and any('last saved snapshot' in warning for warning in report['warnings'])
    run(valid, 2, extra=('--unknown',))
    print('PASS: history, shrink guard, invalid JSON/source rejection, invalid action removal, stale-KB fallback, invalid arguments; no model calls')
    print(f'Fixtures: {base}')


if __name__ == '__main__':
    main()
