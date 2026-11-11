"""Offscreen company fixtures. Never read the owner's meeting or application folders."""
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path
import argparse
import hashlib
import json
import os
import shutil
import subprocess
import tempfile
from prepare_people_fixture import prepare

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / 'artifacts/companies'
ENV = os.environ.copy()
ENV['PATH'] = r'E:\Personal-Projects\tools\hex-meetings-gnu\w64devkit\bin;' + ENV['PATH']


def fixture(empty=False):
    state = Path(tempfile.mkdtemp(prefix='fixture-', dir=OUT / 'state'))
    meetings = prepare(state)
    for name in ['appdata', 'localappdata', 'peers']:
        (state / name).mkdir()
    (state / 'config.json').write_text(json.dumps(dict(version=1, features={k: True for k in ['tasks', 'memory', 'daily_check', 'projects', 'other_devices']})))
    shutil.copy2(ROOT / 'tools/ui-fixtures/hex.json', state / 'hex-fixture.json')
    records = {'fixture-northwind': dict(id='fixture-northwind', name='Northwind', domain='northwind.example', notes='Prototype workshop partner. Keep the next sample small.', aliases=[]), 'fixture-lumen': dict(id='fixture-lumen', name='Lumen', domain=None, notes=None, aliases=[])}
    people = {} if empty else {name: dict(company_id='fixture-northwind', email=None) for name in ['dana ortiz', 'leo park']}
    (state / 'companies.json').write_text(json.dumps(dict(catalog=dict(records=records, people=people, migrated=True), undo=None)))
    for path in meetings.glob('*/english-summary.json'):
        data = json.loads(path.read_text())
        if 'Old' not in data.get('title', ''):
            data['topics'] = [dict(title='Prototype', points=[dict(type='decided', text='Use the small prototype for the next workshop.', at=['00:01:00'])])]
            path.write_text(json.dumps(data))
    return state, meetings


def render(exe, name, scene, width=1280, theme='light', state=None, events=None):
    if state is None:
        state, meetings = fixture(name.startswith('empty-'))
    else:
        meetings = state / 'people-meetings'
    env = ENV | {'APPDATA': str(state / 'appdata'), 'LOCALAPPDATA': str(state / 'localappdata'), 'SIDENOTE_STATE_DIR': str(state)}
    base = OUT / f'{name}-{width}-{theme}'
    args = [str(exe), '--meetings-dir', str(meetings), '--data-dir', str(state), '--peers-dir', str(state / 'peers'), '--scene', scene, '--theme', theme, '--size', f'{width}x800', '--scale', '1', '--snap', str(base)+'.png', '--tree', str(base)+'.json']
    if events:
        script = OUT / 'scripts' / (base.name + '.json')
        script.parent.mkdir(exist_ok=True)
        script.write_text(json.dumps(dict(events=events + [dict(type='mark', name='end', t=max(e['t'] for e in events)+1)])))
        args += ['--script', str(script), '--at', 'mark:end']
    with Path(str(base)+'.log').open('w') as log:
        subprocess.run(args, cwd=ROOT, env=env, stdout=log, stderr=log, creationflags=subprocess.CREATE_NO_WINDOW, timeout=120, check=True)
    tree = json.loads(Path(str(base)+'.json').read_text(encoding='utf-8-sig'))
    assert not tree['warnings'], (base.name, tree['warnings'])
    print('Rendered '+base.name, flush=True)
    return dict(path=str(base)+'.png', tree=str(base)+'.json', state=str(state), scene=scene, width=width, theme=theme)


def main():
    p = argparse.ArgumentParser()
    p.add_argument('exe', type=Path)
    p.add_argument('--quick', action='store_true')
    p.add_argument('--settings', action='store_true')
    a = p.parse_args()
    OUT.mkdir(parents=True, exist_ok=True)
    (OUT / 'state').mkdir(exist_ok=True)
    cases = [(tab, 'company-'+tab) for tab in ['overview', 'people', 'meetings', 'decisions']]
    cases += [('empty-'+tab, 'company-empty-'+tab) for tab in ['overview', 'people', 'meetings', 'decisions']]
    cases += [('external', 'company-external'), ('merge', 'company-merge')]
    shots = [(name, scene, width, theme) for name, scene in cases for width in ([1280] if a.quick else [1280, 390]) for theme in (['light'] if a.quick else ['light', 'dark'])]
    if a.settings:
        settings = ['general', 'recording', 'summaries', 'tasks', 'prompts', 'people', 'hex', 'helpers', 'speech', 'helper-advanced', 'appearance', 'sync', 'updates', 'search', 'recording-expanded', 'summaries-expanded', 'tasks-expanded', 'hex-expanded', 'helper-advanced-expanded', 'sync-expanded', 'updates-expanded', 'helpers-expanded']
        shots = [('settings-'+scene, 'settings-'+scene, 1280, 'light') for scene in settings]
        shots += [('settings-team-editor', 'people-team-editor', 1280, 'light')]
    def run(shot):
        events = None
        if shot[0] == 'settings-search':
            events = [dict(type='tap', key='hex-filter', t=.3), dict(type='type', text='silence', t=.5), dict(type='key', name='Enter', t=1.5)]
        if shot[0] == 'settings-team-editor':
            events = [dict(type='tap', key='team-advanced', t=.8)]
        return render(a.exe.resolve(), *shot, events=events)
    with ThreadPoolExecutor(max_workers=2) as pool:
        rows = list(pool.map(run, shots))
    (OUT / ('legacy-settings-index.json' if a.settings else 'render-index.json')).write_text(json.dumps(dict(binary_sha256=hashlib.sha256(a.exe.read_bytes()).hexdigest(), rows=rows), indent=2))
    print(f'PASS: {len(rows)} offscreen company renders', flush=True)


if __name__ == '__main__':
    main()
