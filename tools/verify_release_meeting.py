"""Headless release integration proof with the real Ask thinking picker."""
import argparse
import hashlib
import json
from pathlib import Path

import render_meeting_review as meeting
import render_memory_review as memory
import render_people_review as native
import render_tasks_review as tasks
from PIL import Image, ImageChops


def nodes(row):
    tree = json.loads(Path(row['tree']).read_text(encoding='utf-8-sig'))
    assert not tree['warnings'], (row['path'], tree['warnings'])
    assert Image.open(row['path']).size == (row['width'], 800)
    return {n['key']: n for n in tree['elements'] if n.get('key')}


def aligned(row, sheet=False):
    n = nodes(row)
    pill, lead = n['ask-pill']['rect'], n['meeting-transcript']['rect']
    thinking = n['ask-thinking']
    assert thinking['label'] == 'Thinking: Low'
    assert n['ask-input']['rect'][2] >= 150, n['ask-input']['rect']
    for key in ['ask-pill', 'meeting-transcript', 'ask-thinking']:
        x, _, w, _ = n[key]['rect']
        assert x >= 0 and x + w <= row['width'], (key, n[key]['rect'])
    if row['width'] == 1280:
        assert abs(lead[1] + lead[3] / 2 - pill[1] - pill[3] / 2) < 1
        assert lead[0] + lead[2] + 8 <= pill[0]
    else:
        assert lead[1] + lead[3] <= pill[1]
    if sheet:
        rect = n['transcript-sheet']['rect']
        assert abs(rect[1] + rect[3] + 10 - min(pill[1], lead[1])) < 1


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('exe', type=Path)
    parser.add_argument('--out', type=Path, default=Path('artifacts/dev13/renders'))
    args = parser.parse_args()
    exe, out = args.exe.resolve(), args.out.resolve()
    out.mkdir(parents=True, exist_ok=True)
    native.OUT = meeting.OUT = memory.OUT = tasks.OUT = out
    rows = []
    for width in [1280, 640]:
        for theme in ['light', 'dark']:
            for name, case in [('meeting', 'finished'), ('meeting-source-hover', 'source-hover'),
                               ('meeting-transcript-sheet', 'transcript-sheet')]:
                row = meeting.render(exe, name, case, width, theme)
                aligned(row, sheet=case == 'transcript-sheet')
                rows.append(row)
            row = meeting.render(exe, 'meeting-ask-picker', width=width, theme=theme,
                                 events=[native.tap('ask-thinking')])
            aligned(row)
            assert 'ask-thinking-menu-item2' in nodes(row)
            rows.append(row)
            row = meeting.render(exe, 'meeting-sheet-picker', width=width, theme=theme,
                                 events=[native.tap('meeting-transcript'), native.tap('ask-thinking', .7)])
            aligned(row, sheet=True)
            assert 'ask-thinking-menu-item2' in nodes(row)
            rows.append(row)
    for theme in ['light', 'dark']:
        rows.append(native.render(exe, 'ask-in-meeting', 'ask-in-meeting', theme=theme))
        rows.append(tasks.render(exe, 'tasks', 'tasks-v2-status', 1280, theme))
        rows.append(memory.render(exe, 'memory', theme=theme))
        rows.append(native.render(exe, 'people-our-team', 'people-our-team', theme=theme))
        rows.append(native.render(exe, 'annotation-note', 'annotation-note', theme=theme))
        rows.append(native.render(exe, 'models-and-agents-top', 'settings-helpers', theme=theme))
        rows.append(native.render(exe, 'models-and-agents', 'settings-helpers', theme=theme,
                                  events=[dict(type='scroll', key='hex-scroll', dy=-600, dur=.2, t=.3)]))
        rows.append(native.render(exe, 'settings-general', 'settings-general', theme=theme))
    for width in [1280, 640]:
        row = meeting.render(exe, 'meeting-draft-escape', width=width,
                             events=[native.tap('meeting-transcript'), native.tap('ask-input', .6),
                                     dict(type='type', text='Keep this draft question', t=.9),
                                     native.key('Escape', 1.2)])
        n = nodes(row)
        assert 'transcript-sheet' not in n
        tree = json.loads(Path(row['tree']).read_text(encoding='utf-8-sig'))
        assert any(e.get('text') == 'Keep this draft question' for e in tree['elements'])
        rows.append(row)
    for row in rows:
        nodes(row)
    (out / 'render-index.json').write_text(json.dumps(dict(
        binary_sha256=hashlib.sha256(exe.read_bytes()).hexdigest(), rows=rows), indent=2))
    comparisons = []
    for row in rows:
        path = Path(row['path'])
        for branch in ['meeting', 'dev12']:
            reference = out.parent / 'reference' / branch / path.name
            if reference.exists():
                diff = ImageChops.difference(Image.open(path).convert('RGB'),
                                            Image.open(reference).convert('RGB'))
                bbox = diff.getbbox()
                comparisons.append(dict(branch=branch, image=path.name, bbox=bbox,
                                        identical=bbox is None))
                if bbox:
                    folder = out.parent / 'diffs'
                    folder.mkdir(exist_ok=True)
                    diff.save(folder / (branch + '-' + path.name))
    (out.parent / 'comparisons.json').write_text(json.dumps(comparisons, indent=2))
    print(f'PASS: {len(rows)} renders, real picker alignment, sheet anchoring and draft Escape; '
          f'{len(comparisons)} reference comparisons', flush=True)


if __name__ == '__main__':
    main()
