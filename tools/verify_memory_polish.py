"""Exercise real Memory menus, local persistence, removals, collapse and navigation offscreen."""
import argparse
import json
import tempfile
from pathlib import Path
import render_people_review as native
from prepare_memory_fixture import prepare

ROOT = Path(__file__).resolve().parents[1]

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('exe', type=Path)
    args = parser.parse_args()
    out = ROOT / 'artifacts/ui-polish/interactions'
    (out / 'state').mkdir(parents=True, exist_ok=True)
    native.OUT = out
    state = Path(tempfile.mkdtemp(prefix='filters-', dir=out / 'state'))
    meetings = prepare(state)
    tap = native.tap
    def render(name, events):
        shot = native.render(args.exe.resolve(), name, 'memory', 1280, 'light', events, state, meetings)
        tree = json.loads(Path(shot['tree']).read_text(encoding='utf-8-sig'))
        assert not tree['warnings'], tree['warnings']
        return {node['key']: node for node in tree['elements'] if node.get('key')}
    nodes = render('select-two', [tap('memory-browse'), tap('memory-filter-type', .7), tap('memory-filter-type-menu-item4', 1.1), tap('memory-filter-project', 1.5), tap('memory-filter-project-menu-item1', 1.9)])
    assert 'memory-clear-type' in nodes and 'memory-clear-project' in nodes
    prefs_path = state / 'local/ui.json'
    prefs = json.loads(prefs_path.read_text())
    assert prefs['memory_kind'] == 'decision', prefs
    assert prefs['memory_project'] == 'Northwind Field Guide', prefs
    nodes = render('reopen-saved', [tap('memory-browse')])
    assert 'memory-clear-type' in nodes and 'memory-clear-project' in nodes
    nodes = render('collapse', [tap('memory-browse'), tap('memory-collapse-Decision', .7)])
    assert 'memory-collapse-Decision' in nodes and 'memory-group-Decision' not in nodes
    nodes = render('remove-one', [tap('memory-browse'), tap('memory-clear-type', .7)])
    assert 'memory-clear-type' not in nodes and 'memory-clear-project' in nodes
    nodes = render('clear-all', [tap('memory-browse'), tap('memory-clear-all', .7)])
    assert 'memory-clear-all' not in nodes
    prefs = json.loads(prefs_path.read_text())
    assert prefs['memory_kind'] is None and prefs['memory_project'] is None
    nodes = render('empty-retired', [tap('memory-browse'), tap('memory-filter-state', .7), tap('memory-filter-state-menu-item2', 1.1)])
    assert 'memory-clear-state' in nodes and 'memory-filter-type' in nodes
    assert not any(key.startswith('memory-group-') for key in nodes)
    nodes = render('breadcrumb', [tap('memory-browse'), tap('memory-breadcrumb-parent', .7)])
    assert 'memory-browse' in nodes and 'memory-filter-type' not in nodes
    print('PASS: immediate menu selection, two filters, persistence after restart, collapse, individual removal, Clear all, empty results and breadcrumb navigation')

if __name__ == '__main__':
    main()
