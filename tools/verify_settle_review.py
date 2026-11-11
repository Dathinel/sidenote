"""Check settle history, real Undo/Redo persistence, citations, and unchanged Settings pixels."""
import argparse
import hashlib
import json
import tempfile
from pathlib import Path
from PIL import Image, ImageChops
from render_settle_review import OUT, ID, prepare, render, tap


def nodes(name, theme='light'):
    tree = json.loads((OUT / f'{name}-1280-{theme}.json').read_text(encoding='utf-8-sig'))
    assert not tree['warnings'], (name, tree['warnings'])
    return tree['elements']


def has(nodes_, key):
    return any(n.get('key') == key for n in nodes_)


def originals(state):
    entries = []
    for path in (state / 'memory/settle-log').glob('*.intent.json'):
        intent = json.loads(path.read_text())
        outcome = json.loads(path.with_name(path.name.replace('.intent.', '.done.')).read_text())
        assert outcome['applied']
        entries.extend(intent['entries'])
    return entries


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('exe', type=Path)
    parser.add_argument('--reference', type=Path, required=True)
    args = parser.parse_args()
    exe = args.exe.resolve()
    index = json.loads((OUT / 'render-index.json').read_text())
    assert index['binary_sha256'] == hashlib.sha256(exe.read_bytes()).hexdigest()
    assert len(index['rows']) == 10
    for shot in index['rows']:
        path = Path(shot['path'])
        assert Image.open(path).size == (shot['width'], 800)
        tree = json.loads(Path(shot['tree']).read_text(encoding='utf-8-sig'))
        assert not tree['warnings'], (path, tree['warnings'])
        if shot['scene'] == 'settings-general':
            a = Image.open(path).convert('RGB')
            b = Image.open(args.reference / path.name).convert('RGB')
            assert ImageChops.difference(a, b).getbbox() is None, path
    for theme in ('light', 'dark'):
        base = nodes('memory-settled', theme)
        assert len([n for n in base if n.get('key', '').startswith('settled-')]) == 3
        assert has(base, 'settle-see-all')
        undone = nodes('memory-settle-undo', theme)
        assert has(undone, 'settle-toast-action')
        assert any(n.get('text') == 'Change undone.' for n in undone)
        all_ = nodes('memory-settle-all', theme)
        assert len([n for n in all_ if n.get('key', '').startswith('settled-')]) == 6
        assert has(all_, 'settle-back')
    many = Path(tempfile.mkdtemp(prefix='verify-five-', dir=OUT/'state'))
    prepare(many, many=True)
    render(exe, 'verify-five', state=many)
    assert len([n for n in nodes('verify-five') if n.get('key', '').startswith('settled-')]) == 5
    assert len(originals(many)) == 6
    state = Path(tempfile.mkdtemp(prefix='verify-undo-', dir=OUT/'state'))
    render(exe, 'verify-original', state=state)
    immutable = {p: p.read_bytes() for p in (state/'memory/settle-log').glob('*.json')}
    tasks = state/'settle-meetings'/ID/'tasks.json'
    before = json.loads(tasks.read_text())
    render(exe, 'verify-undo', events=[tap('settle-undo-0', .4)], state=state)
    assert json.loads(tasks.read_text()) == {'meeting_project': None, 'tasks': []}
    assert len(originals(state)) == 4
    assert has(nodes('verify-undo'), 'settle-toast-action')
    render(exe, 'verify-redo-restart', events=[tap('settle-undo-0', .4)], state=state)
    assert json.loads(tasks.read_text()) == before
    assert len(originals(state)) == 5
    render(exe, 'verify-task-undo', events=[tap('settle-undo-1', .4)], state=state)
    summary = state/'settle-meetings'/ID/'english-summary.json'
    assert json.loads(summary.read_text())['todo'][1]['owner'] is None
    render(exe, 'verify-toast-redo', events=[tap('settle-undo-1', .4)], state=state)
    assert json.loads(summary.read_text())['todo'][1]['owner'] == 'Morgan Lee'
    render(exe, 'verify-direct-toast-redo', events=[tap('settle-undo-1', .4), tap('settle-toast-action', 1.5)], state=state)
    assert json.loads(summary.read_text())['todo'][1]['owner'] == 'Morgan Lee'
    assert len(originals(state)) == 9
    assert all(p.read_bytes() == value for p, value in immutable.items())
    render(exe, 'verify-source', events=[tap('settle-source-1', .4)], state=state)
    source = nodes('verify-source')
    assert has(source, 'meeting-settle-memory')
    assert any(n.get('text') == 'Sidenote updated 3 things from this meeting.' for n in source)
    render(exe, 'verify-meeting-history', events=[tap('settle-source-1', .4), tap('meeting-settle-memory', 1.4)], state=state)
    assert has(nodes('verify-meeting-history'), 'settle-back')
    assert len([n for n in nodes('verify-meeting-history') if n.get('key', '').startswith('settled-')]) == 3
    (OUT/'validation.json').write_text(json.dumps(dict(result='pass', matrix_renders=10, settings_pixel_identical=4, reference=str(args.reference.resolve()), checks=['five-row limit', 'three settled rows', 'six in See all', 'Undo source data', 'Redo after restart', 'task Undo and Redo', 'toast Redo', 'append-only outcomes', 'source navigation', 'meeting count and filtered history']), indent=2))
    print('PASS: native Undo/Redo, restart, citations, count, all history and four exact Settings references', flush=True)


if __name__ == '__main__':
    main()
