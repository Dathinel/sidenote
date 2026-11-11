"""Native offscreen recording-guard proofs, using invented recordings only."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / 'artifacts/rec-guard'


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('exe', type=Path)
    args = parser.parse_args()
    exe = args.exe.resolve()
    OUT.mkdir(parents=True, exist_ok=True)
    state = OUT / 'render-state'
    meetings = state / 'meetings'
    for folder in ['appdata', 'localappdata', 'peers']:
        (state / folder).mkdir(parents=True, exist_ok=True)
    (state / 'config.json').write_text(json.dumps(dict(version=1, notes=dict(engine='off'), features=dict(tasks=True, memory=True))), encoding='utf-8')
    for ident, duration, words in [('1793946600000-1', 29999, 25), ('1794033000000-1', 96000, 0), ('1794119400000-1', 30000, 25)]:
        folder = meetings / ident
        folder.mkdir(parents=True, exist_ok=True)
        (folder / 'manifest.json').write_text(json.dumps(dict(status='complete', created_at_ms=int(ident.split('-')[0]), duration_ms=duration, title='Synthetic recording')), encoding='utf-8')
        (folder / 'transcript.ndjson').write_text(json.dumps(dict(text=' '.join(['synthetic'] * words))), encoding='utf-8')
    env = os.environ.copy()
    env.update(APPDATA=str(state / 'appdata'), LOCALAPPDATA=str(state / 'localappdata'), SIDENOTE_STATE_DIR=str(state))
    env['PATH'] = r'E:\Personal-Projects\tools\hex-meetings-gnu\w64devkit\bin;' + env['PATH']
    decisions = state / 'local/recording-guard'
    decisions.mkdir(parents=True, exist_ok=True)
    for ident in ['1793946600000-1', '1794033000000-1', '1794119400000-1']:
        (decisions / f'{ident}.json').write_text(json.dumps(dict(short=False, detected_at=None, kept=False, work_queued=False)), encoding='utf-8')
    rows = []
    for width in [1280, 390]:
        for scene in ['short-recordings', 'home-collapsed']:
            base = OUT / f'{scene}-{width}'
            command = [str(exe), '--meetings-dir', str(meetings), '--data-dir', str(state), '--peers-dir', str(state / 'peers'), '--scene', scene, '--theme', 'light', '--size', f'{width}x800', '--scale', '1', '--snap', str(base) + '.png', '--tree', str(base) + '.json']
            with base.with_suffix('.log').open('w') as log:
                subprocess.run(command, cwd=ROOT, env=env, stdout=log, stderr=log, creationflags=subprocess.CREATE_NO_WINDOW, timeout=120, check=True)
            tree = json.loads(base.with_suffix('.json').read_text(encoding='utf-8'))
            raw = json.dumps(tree)
            if scene == 'short-recordings':
                assert 'keep-recording-1793946600000-1' in raw
                assert 'keep-recording-1794033000000-1' in raw
                assert 'keep-recording-1794119400000-1' not in raw
                assert 'Short recordings (under 30 seconds or 25 words) are hidden and not processed.' in raw
            else:
                assert 'meeting-1793946600000-1' not in raw
                assert 'meeting-1794033000000-1' not in raw
                assert 'meeting-1794119400000-1' in raw
            rows.append(dict(path=str(base) + '.png', tree=str(base) + '.json', width=width, scene=scene))
            print('PASS ' + base.name, flush=True)
    config = json.loads((state / 'config.json').read_text(encoding='utf-8'))
    config['notes'] = dict(engine='app', helper=None, auto=False)
    (state / 'config.json').write_text(json.dumps(config), encoding='utf-8')
    script = OUT / 'keep-event.json'
    script.write_text(json.dumps(dict(events=[dict(type='tap', key='keep-recording-1793946600000-1', t=.3), dict(type='mark', name='end', t=1.0)])), encoding='utf-8')
    base = OUT / 'kept-390'
    command = [str(exe), '--meetings-dir', str(meetings), '--data-dir', str(state), '--peers-dir', str(state / 'peers'), '--scene', 'short-recordings', '--theme', 'light', '--size', '390x800', '--scale', '1', '--snap', str(base) + '.png', '--tree', str(base) + '.json', '--script', str(script), '--at', 'mark:end']
    with base.with_suffix('.log').open('w') as log:
        subprocess.run(command, cwd=ROOT, env=env, stdout=log, stderr=log, creationflags=subprocess.CREATE_NO_WINDOW, timeout=120, check=True)
    decision = json.loads((decisions / '1793946600000-1.json').read_text(encoding='utf-8'))
    assert decision['short'] is False and decision['kept'] is True and decision['work_queued'] is True
    assert isinstance(decision['detected_at'], int)
    kept_tree = base.with_suffix('.json').read_text(encoding='utf-8')
    assert 'keep-recording-1793946600000-1' not in kept_tree
    assert 'keep-recording-1794033000000-1' in kept_tree
    jobs = list((state / 'local/notes-jobs').glob('*.json'))
    assert [job.stem for job in jobs] == ['1793946600000-1']
    assert json.loads(jobs[0].read_text(encoding='utf-8'))['state'] in ['queued', 'no_helper']
    rows.append(dict(path=str(base) + '.png', tree=str(base) + '.json', width=390, scene='short-recordings', action='Keep'))
    print('PASS Keep persisted and removed only the chosen recording', flush=True)
    (OUT / 'render-index.json').write_text(json.dumps(dict(binary_sha256=hashlib.sha256(exe.read_bytes()).hexdigest(), rows=rows), indent=2), encoding='utf-8')


if __name__ == '__main__':
    main()
