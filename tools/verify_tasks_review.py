"""Assert Tasks layout and real offscreen interactions against the review fixture."""
import argparse
import json
import tempfile
from datetime import date, timedelta
from pathlib import Path
from PIL import Image, ImageChops
from render_tasks_review import OUT, render, tap


def elements(name, width=1280, theme='light'):
    tree = json.loads((OUT / f'{name}-{width}-{theme}.json').read_text(encoding='utf-8-sig'))
    assert not tree['warnings'], (name, tree['warnings'])
    return tree['elements']


def keyed(nodes, key):
    return next(n for n in nodes if n.get('key') == key)


def key(name, t):
    return {'t': t, 'type': 'key', 'name': name}


def inside(node, width, height=800):
    x, y, w, h = node['rect']
    assert x >= 0 and y >= 0 and x+w <= width+.5 and y+h <= height+.5, node


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('exe', type=Path)
    parser.add_argument('--reference', type=Path, required=True)
    args = parser.parse_args()
    exe = args.exe.resolve()
    index = json.loads((OUT / 'render-index.json').read_text())
    assert len(index['rows']) == 24, len(index['rows'])
    for shot in index['rows']:
        path = Path(shot['path'])
        assert Image.open(path).size == (shot['width'], 800), path
        nodes = elements(path.stem.rsplit('-', 2)[0], shot['width'], shot['theme'])
        if shot['scene'].startswith('settings-'):
            reference = args.reference / path.name
            assert ImageChops.difference(Image.open(path).convert('RGB'), Image.open(reference).convert('RGB')).getbbox() is None, path
    for width in (1280, 640):
        for theme in ('light', 'dark'):
            folded = elements('tasks', width, theme)
            expanded = elements('tasks-subtasks', width, theme)
            assert not any(n.get('key', '').startswith('subtick-') for n in folded)
            assert len([n for n in expanded if n.get('key', '').startswith('subtick-')]) == 1
            for n in folded:
                if '/ttask-' in n.get('path', '') and n.get('role') == 'text':
                    assert n['rect'][3] <= 42.1, n
            detail = elements('task-detail', width, theme)
            for name in ('task-source-link', 'pdone', 'task-status', 'powner', 'pdue-open'):
                inside(keyed(detail, name), width)
            assert not any(n.get('key') in ['pdue-today', 'pdue-tomorrow', 'pdue-week', 'pdue-none', 'pdue', 'task-text-save'] for n in detail)
            labels = {n.get('label') for n in elements('tasks-filters', width, theme) if n.get('role', '').startswith('menuitem')}
            assert {'Everyone', 'Mine', 'Include unassigned', 'Clear filters'} <= labels
    print('PASS: 24 dimensions/trees, 8 pixel-identical Settings references, folded subtasks, two-line title bounds, pinned source/footer, on-demand editors and filter choices', flush=True)

    render(exe, 'status-choices', 'tasks-v2-status', 1280, 'light', [tap('task-status-filter')])
    assert {n.get('label') for n in elements('status-choices') if n.get('role', '').startswith('menuitem')} == {'Open', 'Done', 'All'}
    state = Path(tempfile.mkdtemp(prefix='filter-persistence-', dir=OUT / 'state'))
    render(exe, 'filter-saved', 'tasks-v2-status', 1280, 'light', [tap('task-filters'), tap('tasks-filters-menu-item2', .6), tap('task-filters', 1), tap('tasks-filters-menu-item3', 1.3), tap('task-status-filter', 1.6), tap('tasks-status-menu-item2', 1.9)], state)
    prefs = json.loads((state / 'local/ui.json').read_text())
    assert prefs['tasks_mine'] and prefs['tasks_include_unassigned'] and prefs['tasks_done'] == 'All'
    render(exe, 'filter-restored', 'tasks-v2-status', 1280, 'light', None, state)
    assert keyed(elements('filter-restored'), 'task-filters')['label'] == 'Filters: Mine + unassigned'
    assert keyed(elements('filter-restored'), 'task-status-filter')['label'] == 'All'
    render(exe, 'keyboard-panel', 'tasks-v2-status', 1280, 'light', [key('j', .3), key('Enter', .6), key('Escape', 1)])
    nodes = elements('keyboard-panel')
    assert not any(n.get('key') == 'tasks-panel' for n in nodes)
    render(exe, 'keyboard-reopen', 'tasks-v2-status', 1280, 'light', [key('j', .3), key('Enter', .6), key('Escape', 1), key('Enter', 1.3)])
    assert keyed(elements('keyboard-reopen'), 'task-status')['label'] == 'To do'
    render(exe, 'focus-return', 'tasks-v2-panel', 1280, 'light', [tap('task-status'), key('Escape', .6), key('Escape', 1)])
    assert keyed(elements('focus-return'), 'ttask-1800000000000-1-0').get('focused'), 'Escape did not return panel focus to the row'
    render(exe, 'keyboard-filter', 'tasks-v2-status', 1280, 'light', [tap('task-filters'), key('Down', .6), key('Down', .8), key('Enter', 1.1)])
    assert keyed(elements('keyboard-filter'), 'task-filters')['label'] == 'Filters: Mine'
    render(exe, 'keyboard-date', 'tasks-v2-status', 1280, 'light', [key('j', .3), key('d', .6)])
    assert any(n.get('key') == 'pdue-tomorrow' for n in elements('keyboard-date'))
    due_shot = render(exe, 'due-edit', 'tasks-v2-panel', 1280, 'light', [tap('pdue-open'), tap('pdue-tomorrow', .6)])
    operations = [json.loads(p.read_text()) for p in (Path(due_shot['state']) / 'state').rglob('*.json')]
    expected_due = (date.today() + timedelta(days=1)).isoformat()
    assert any(op.get('store') == 'task-overrides.json' and any(change.get('key') == ['tasks', '1800000000000-1', 'id:draft'] and change.get('value', {}).get('due') == expected_due for change in op.get('changes', [])) for op in operations), 'Due edit was not persisted'
    render(exe, 'due-restored', 'tasks-v2-panel', 1280, 'light', [tap('pdue-open')], Path(due_shot['state']))
    assert any(n.get('text') == 'Due tomorrow.' for n in elements('due-restored'))
    render(exe, 'owner-edit', 'tasks-v2-panel', 1280, 'light', [tap('powner'), tap('powner-menu-item0', .6)])
    assert keyed(elements('owner-edit'), 'powner')['label'] == 'Owner: Me'
    render(exe, 'status-edit', 'tasks-v2-panel', 1280, 'light', [tap('task-status'), tap('task-status-menu-item2', .6)])
    assert keyed(elements('status-edit'), 'task-status')['label'] == 'In review'
    subtask_shot = render(exe, 'subtasks-edit', 'tasks-v2-panel', 1280, 'light', [tap('panel-sub-share'), tap('subtask-add-open', .8), tap('subtask-title', 1.1), {'t': 1.4, 'type': 'type', 'text': 'Fold the sample pack'}, tap('subtask-add', 2.2)])
    nodes = elements('subtasks-edit')
    assert any(n.get('text') == 'Sub-tasks 1/2' for n in nodes)
    operations = [json.loads(p.read_text()) for p in (Path(subtask_shot['state']) / 'state').rglob('*.json')]
    assert any(op.get('store') == 'task-overrides.json' and any(change.get('value', {}).get('subtask_ticks', {}).get('sub:5:draft:5:share') is True for change in op.get('changes', [])) for op in operations), 'Subtask tick was not persisted'
    assert any(n.get('text') == 'Fold the sample pack' for n in nodes)
    render(exe, 'full-titles', 'tasks-v2-status', 640, 'light', [tap('show-full-titles')])
    for n in elements('full-titles', 640):
        if '/ttask-' in n.get('path', '') and n.get('role') == 'text':
            assert n['rect'][3] <= 42.1, n
    render(exe, 'hidden-filter', 'tasks-v2-status', 1280, 'light', [tap('task-filters'), tap('tasks-filters-menu-item7', .6)])
    labels = {n.get('label') for n in elements('hidden-filter') if n.get('role', '').startswith('menuitem')}
    assert {'Older than 7 days', 'Older than 14 days', 'Older than 30 days', 'Keep all'} <= labels
    hidden_shot = render(exe, 'hidden-confirm', 'tasks-v2-status', 1280, 'light', [tap('task-filters'), tap('tasks-filters-menu-item7', .6), tap('cleared-menu-item0', 1.4)])
    assert any(n.get('key') == 'clear-confirm' for n in elements('hidden-confirm'))
    assert not (Path(hidden_shot['state']) / 'tasks-settings.json').exists(), 'Cutoff changed before confirmation'
    assert not any(json.loads(p.read_text()).get('store') == 'tasks-settings.json' for p in (Path(hidden_shot['state']) / 'state').rglob('*.json')), 'Cutoff journal changed before confirmation'
    render(exe, 'completion', 'tasks-v2-panel', 1280, 'light', [tap('pdone')])
    assert keyed(elements('completion'), 'pdone')['text'] == 'Mark not done'
    render(exe, 'description-escape', 'tasks-v2-panel', 1280, 'light', [tap('pdescription'), key('Escape', .6)])
    assert not any(n.get('key') == 'tasks-panel' for n in elements('description-escape'))
    assert keyed(elements('description-escape'), 'ttask-1800000000000-1-0').get('focused')
    # Script typing is paced; finish each string before the next click.
    render(exe, 'text-edit', 'tasks-v2-panel', 1280, 'light', [tap('ptitle'), {'t': .6, 'type': 'key', 'name': 'a', 'cmd': True}, {'t': .9, 'type': 'type', 'text': 'Review the paper samples'}, tap('pdescription', 3.0), {'t': 3.3, 'type': 'key', 'name': 'a', 'cmd': True}, {'t': 3.6, 'type': 'type', 'text': 'Bring the marked copy on Friday.'}, tap('task-text-save', 6.0)])
    nodes = elements('text-edit')
    assert any(n.get('text') == 'Review the paper samples' for n in nodes)
    assert any(n.get('text') == 'Bring the marked copy on Friday.' for n in nodes)
    assert not any(n.get('key') == 'task-text-save' for n in nodes)
    render(exe, 'text-cancel', 'tasks-v2-panel', 1280, 'light', [tap('ptitle'), {'t': .6, 'type': 'key', 'name': 'a', 'cmd': True}, {'t': .9, 'type': 'type', 'text': 'Discard this title'}, tap('task-text-cancel', 3.0)])
    nodes = elements('text-cancel')
    assert keyed(nodes, 'ptitle')['text'] == 'Review the field guide'
    assert not any(n.get('key') == 'task-text-save' for n in nodes)
    render(exe, 'search', 'tasks-v2-status', 1280, 'light', [tap('task-search'), {'t': .6, 'type': 'type', 'text': 'invitation'}])
    rows = [n for n in elements('search') if n.get('key', '').startswith('ttask-')]
    assert len(rows) == 1 and rows[0]['key'].endswith('-2')
    render(exe, 'source-navigation', 'tasks-v2-panel', 1280, 'light', [tap('task-source-link')])
    assert not any(n.get('key') == 'tasks-pane' for n in elements('source-navigation'))
    (OUT / 'validation.json').write_text(json.dumps({'result': 'pass', 'matrix_renders': 24, 'pixel_identical_settings': 8, 'interactions': ['status choices', 'filter persistence and restart', 'keyboard open and reopen', 'focus return', 'keyboard Filters', 'date shortcut', 'due edit', 'owner edit', 'status edit', 'subtask tick and add', 'full titles', 'hidden-task filters and confirmation', 'completion', 'description Escape', 'title and description save', 'text Cancel', 'search', 'source navigation']}, indent=2))
    print('PASS: status choices, persisted filters and restart, keyboard open/Escape/focus and date shortcut, due/owner/status/subtask edits, source navigation', flush=True)


if __name__ == '__main__':
    main()
