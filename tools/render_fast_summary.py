"""Offscreen draft and final proofs, using only an invented meeting."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / 'artifacts/fast-summary'
ID = '1793955600000-1'
NOTICE = 'Draft from the English transcript. Improving with Telugu and Hindi.'


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('exe', type=Path)
    args = parser.parse_args()
    exe = args.exe.resolve()
    OUT.mkdir(parents=True, exist_ok=True)
    rows = []
    for width in [1280, 390]:
        state = OUT / f'render-state-{width}'
        meetings = state / 'meetings'
        folder = meetings / ID
        for directory in [folder, state / 'appdata', state / 'localappdata', state / 'peers']:
            directory.mkdir(parents=True, exist_ok=True)
        (state / 'config.json').write_text(json.dumps(dict(version=1, notes=dict(engine='scripts', script='C:/synthetic.ps1'), features=dict(tasks=True, memory=True))), encoding='utf-8')
        (folder / 'manifest.json').write_text(json.dumps(dict(status='complete', created_at_ms=1793955600000, duration_ms=1800000, title='Synthetic planning meeting')), encoding='utf-8')
        transcript = dict(start_ms=1000, end_ms=1800000, source='microphone', text=' '.join(['synthetic'] * 30))
        (folder / 'transcript.ndjson').write_text(json.dumps(transcript), encoding='utf-8')
        draft = dict(schema=2, title='Synthetic planning meeting', context=['English draft: the team reviewed the workshop plan.'], topics=[dict(title='Workshop plan', points=[dict(type='fact', text='A paper prototype will be reviewed next week.', who=None, at=[])])], todo=[], to_settle=[])
        draft_path = state / f'local/draft-summaries/{ID}.json'
        draft_path.parent.mkdir(parents=True, exist_ok=True)
        final_path = folder / 'english-summary.json'
        final_path.unlink(missing_ok=True)
        draft_path.write_text(json.dumps(draft), encoding='utf-8')
        env = os.environ | dict(APPDATA=str(state / 'appdata'), LOCALAPPDATA=str(state / 'localappdata'), SIDENOTE_STATE_DIR=str(state))
        env['PATH'] = r'E:\Personal-Projects\tools\hex-meetings-gnu\w64devkit\bin;' + env['PATH']
        for mode in ['draft', 'final']:
            if mode == 'final':
                final = draft | dict(context=['Final summary: Telugu and Hindi review confirmed the workshop plan.'])
                final_path.write_text(json.dumps(final), encoding='utf-8')
            base = OUT / f'{mode}-{width}'
            command = [str(exe), '--data-dir', str(state), '--meetings-dir', str(meetings), '--peers-dir', str(state / 'peers'), '--scene', 'screens-summary', '--theme', 'light', '--size', f'{width}x800', '--scale', '1', '--snap', str(base) + '.png', '--tree', str(base) + '.json']
            with base.with_suffix('.log').open('w') as log:
                subprocess.run(command, cwd=ROOT, env=env, stdout=log, stderr=log, creationflags=subprocess.CREATE_NO_WINDOW, timeout=180, check=True)
            tree = json.loads(base.with_suffix('.json').read_text(encoding='utf-8-sig'))
            assert not tree['warnings'], tree['warnings']
            elements = tree['elements']
            text = ' '.join(n.get('text', '') for n in elements)
            assert (NOTICE in text) == (mode == 'draft')
            assert ('English draft:' in text) == (mode == 'draft')
            assert ('Final summary:' in text) == (mode == 'final')
            assert 'No summary yet' not in text
            assert draft_path.exists() == (mode == 'draft')
            if mode == 'draft':
                assert not final_path.exists()
                assert not (folder / 'tasks.json').exists()
                assert not (state / 'memory/settle').exists()
                notice = next(n for n in elements if n.get('key') == 'draft-summary-notice')
                x, y, w, h = notice['rect']
                assert 0 <= x and x + w <= width + 1 and 0 <= y and y + h <= 800, notice['rect']
            rows.append(dict(path=str(base) + '.png', tree=str(base) + '.json', width=width, mode=mode))
            print('PASS ' + base.name, flush=True)
    (OUT / 'render-index.json').write_text(json.dumps(dict(binary_sha256=hashlib.sha256(exe.read_bytes()).hexdigest(), rows=rows), indent=2), encoding='utf-8')


if __name__ == '__main__':
    main()
