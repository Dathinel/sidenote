"""Offscreen Tasks review. Uses invented fixtures and never opens a desktop window."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
from concurrent.futures import ThreadPoolExecutor

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / 'artifacts/pages-tasks'
ENV = os.environ.copy()
ENV['PATH'] = r'E:\Personal-Projects\tools\hex-meetings-gnu\w64devkit\bin;' + ENV['PATH']
FLAGS = subprocess.CREATE_NO_WINDOW


def render(exe, name, scene, width, theme, events=None, state=None):
    (OUT / 'state').mkdir(parents=True, exist_ok=True)
    state = state or Path(tempfile.mkdtemp(prefix=f'{name}-{width}-{theme}-', dir=OUT / 'state'))
    state.mkdir(parents=True, exist_ok=True)
    for folder in ['appdata', 'localappdata', 'peers']:
        (state / folder).mkdir(exist_ok=True)
    runenv = ENV | {'APPDATA': str(state / 'appdata'), 'LOCALAPPDATA': str(state / 'localappdata'), 'SIDENOTE_STATE_DIR': str(state)}
    (state / 'config.json').write_text(json.dumps({'version': 1, 'features': {k: True for k in ['tasks', 'memory', 'daily_check', 'projects', 'other_devices']}}))
    (state / 'projects.json').write_text(json.dumps({'projects': ['Northwind Atlas', 'Northwind Field Guide'], 'aliases': {}}))
    shutil.copy2(ROOT / 'tools/ui-fixtures/hex.json', state / 'hex-fixture.json')
    meetings = ROOT / 'tools/ui-fixtures/meetings'
    if scene.startswith('tasks-v2'):
        subprocess.run(['python', str(ROOT / 'tools/prepare_tasks_v2_fixture.py'), str(state)], check=True, env=runenv, creationflags=FLAGS)
        meetings = state / 'tasks-v2-meetings'
    base = OUT / f'{name}-{width}-{theme}'
    args = [str(exe), '--meetings-dir', str(meetings), '--data-dir', str(state), '--peers-dir', str(state / 'peers'), '--snap', str(base) + '.png', '--tree', str(base) + '.json', '--scene', scene, '--theme', theme, '--size', f'{width}x800', '--scale', '1']
    if events:
        script = OUT / f'{name}-{width}-{theme}-script.json'
        script.write_text(json.dumps({'events': events + [{'t': max(e['t'] for e in events) + 1, 'type': 'mark', 'name': 'end'}]}))
        args += ['--script', str(script), '--at', 'mark:end']
    with open(str(base) + '.log', 'w') as log:
        subprocess.run(args, cwd=ROOT, env=runenv, stdout=log, stderr=log, creationflags=FLAGS, check=True, timeout=120)
    assert Path(str(base) + '.png').exists()
    print(f'Rendered {base.name}', flush=True)
    return {'path': str(base) + '.png', 'tree': str(base) + '.json', 'scene': scene, 'theme': theme, 'width': width, 'state': str(state)}


def tap(key, t=.3):
    return {'t': t, 'type': 'tap', 'key': key}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('exe', type=Path)
    parser.add_argument('--quick', action='store_true')
    args = parser.parse_args()
    OUT.mkdir(parents=True, exist_ok=True)
    rows = []
    scenes = [('tasks', 'tasks-v2-status', None), ('tasks-subtasks', 'tasks-v2-status', [tap('subtasks-toggle-1800000000000-1-0')]), ('task-detail', 'tasks-v2-panel', None), ('tasks-filters', 'tasks-v2-status', [tap('task-filters')]), ('settings-general', 'settings-general', None), ('settings-recording', 'settings-recording', None)]
    shots = [(name, scene, width, theme, events) for name, scene, events in scenes
             for width in ([1280] if args.quick else [1280, 640])
             for theme in (['light'] if args.quick else ['light', 'dark'])]
    with ThreadPoolExecutor(max_workers=2) as pool:
        rows = list(pool.map(lambda shot: render(args.exe.resolve(), *shot), shots))
    (OUT / 'render-index.json').write_text(json.dumps({'binary_sha256': hashlib.sha256(args.exe.read_bytes()).hexdigest(), 'rows': rows}, indent=2))
    print(f'PASS: {len(rows)} offscreen renders', flush=True)


if __name__ == '__main__':
    main()
