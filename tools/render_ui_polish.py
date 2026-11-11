"""Reproducible native UI polish matrix with isolated synthetic state."""
import argparse
import json
import shutil
import tempfile
import hashlib
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path
import render_people_review as native
import render_companies as companies
from prepare_memory_fixture import prepare

ROOT = Path(__file__).resolve().parents[1]

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('exe', type=Path)
    parser.add_argument('--baseline', action='store_true')
    parser.add_argument('--width', type=int, choices=[1280, 390])
    args = parser.parse_args()
    out = ROOT / 'artifacts/ui-polish' / ('dev15' if args.baseline else 'current')
    out.mkdir(parents=True, exist_ok=True)
    (out / 'state').mkdir(exist_ok=True)
    native.OUT = out
    companies.OUT = out
    cases = [('updates', 'updates-ready'), ('sync-devices', 'settings-sync-expanded'),
             ('notebook', 'notebook'), ('meeting', 'meeting'),
             ('memory-all-filters', 'memory'), ('memory-all-chip-open', 'memory'),
             ('memory-all-active', 'memory'), ('company-overview', 'company-overview')]
    if not args.baseline:
        cases.insert(0, ('button-sheet', 'button-sheet'))
    jobs = [(name, scene, width, theme) for name, scene in cases
            for width in ([args.width] if args.width else [1280, 390]) for theme in ['light', 'dark']]
    def render(job):
        name, scene, width, theme = job
        if scene.startswith('company-'):
            return companies.render(args.exe.resolve(), name, scene, width, theme, events=None)
        state = Path(tempfile.mkdtemp(prefix='fixture-', dir=out / 'state'))
        events = []
        meetings = None
        if scene == 'memory':
            meetings = prepare(state)
            path = state / 'memory/records.json'
            data = json.loads(path.read_text())
            for i, record in enumerate(data['records']):
                if i < 10:
                    record['type'] = 'person'
                    record['text'] = f'Synthetic colleague {i + 1} owns a workshop sample.'
            path.write_text(json.dumps(data))
            events = [native.tap('memory-browse')]
            if name == 'memory-all-chip-open':
                events += [native.tap('memory-filters' if args.baseline else 'memory-filter-type', .8)]
            elif name == 'memory-all-active' and not args.baseline:
                (state / 'local').mkdir(exist_ok=True)
                (state / 'local/ui.json').write_text(json.dumps(dict(memory_kind='person', memory_project='Northwind Field Guide')))
        if width == 390 and scene == 'notebook':
            events = [native.tap('sidebar-toggle', .2)] + [dict(event, t=event['t'] + .5) for event in events]
        # Native scripts target the offscreen runtime, never the user's desktop.
        return native.render(args.exe.resolve(), name, scene, width, theme, events or None, state, meetings)
    with ThreadPoolExecutor(max_workers=2) as pool:
        rows = list(pool.map(render, jobs))
    index = out / 'render-index.json'
    if args.width and index.exists():
        prior = json.loads(index.read_text())
        rows += [row for row in prior if row['width'] != args.width]
    index.write_text(json.dumps(rows, indent=2))
    (out / 'binary.json').write_text(json.dumps(dict(binary=str(args.exe.resolve()), sha256=hashlib.sha256(args.exe.read_bytes()).hexdigest()), indent=2))
    if not args.baseline:
        for name in ['button-sheet-1280-light', 'memory-all-filters-1280-dark', 'updates-1280-dark']:
            target = 'button-sheet-light' if name == 'button-sheet-1280-light' else name
            shutil.copy2(out / (name + '.png'), out.parent / (target + '.png'))
    print(f'PASS: {len(rows)} native renders in {out}', flush=True)

if __name__ == '__main__':
    main()
