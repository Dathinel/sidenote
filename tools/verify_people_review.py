"""Check native People renders, persistence and team editing in isolated state."""
import argparse
import json
import tempfile
from pathlib import Path
from PIL import Image, ImageChops
from render_people_review import OUT, render, tap, key
from prepare_people_fixture import prepare


def elements(name, width=1280, theme='light'):
    tree = json.loads((OUT / f'{name}-{width}-{theme}.json').read_text(encoding='utf-8-sig'))
    assert not tree['warnings'], (name, tree['warnings'])
    return tree['elements']


def keyed(nodes, value):
    return next(n for n in nodes if n.get('key') == value)


def persons(nodes):
    return {n['key'] for n in nodes if n.get('key', '').startswith('person-') and not n['key'].startswith('person-side-')}


def text(value, t):
    return dict(type='type', text=value, t=t)


def select_all(t):
    return dict(type='key', name='a', cmd=True, t=t)


def scroll(t):
    return dict(type='scroll', key='hex-scroll', dy=-300, dur=.2, t=t)


def verify_member_search(exe):
    state = Path(tempfile.mkdtemp(prefix='many-members-', dir=OUT/'state'))
    prepare(state)
    path = state/'teams.json'
    teams = json.loads(path.read_text())
    teams['teams'][1]['members'] += [f'Extra {i:02}' for i in range(10)]
    path.write_text(json.dumps(teams))
    render(exe, 'many-members', 'people-team-editor', state=state)
    nodes = elements('many-members')
    assert any(n.get('key') == 'team-member-search' for n in nodes)
    render(exe, 'member-search-result', 'people-team-editor', events=[tap('team-member-search'), text('Extra 09', .7), tap('team-member-extra 09', 2), tap('team-save', 2.5)], state=state)
    assert json.loads(path.read_text())['teams'][0]['members'] == ['Alex Chen', 'Jamie Park', 'Extra 09']
    print('PASS: member search appears above 12 people and saves a selected chip', flush=True)


def main():
    p = argparse.ArgumentParser()
    p.add_argument('exe', type=Path)
    p.add_argument('--reference', type=Path, required=True)
    a = p.parse_args()
    exe = a.exe.resolve()
    index = json.loads((OUT / 'render-index.json').read_text())
    assert len(index['rows']) == 24
    for shot in index['rows']:
        path = Path(shot['path'])
        assert Image.open(path).size == (shot['width'], 800)
        nodes = elements(path.stem.rsplit('-', 2)[0], shot['width'], shot['theme'])
        if shot['scene'] == 'settings-general':
            assert ImageChops.difference(Image.open(path).convert('RGBA'), Image.open(a.reference/path.name).convert('RGBA')).getbbox() is None, path
        # Content may scroll vertically; horizontal overflow is never necessary.
        for n in nodes:
            if n.get('key', '').startswith(('person-', 'team-row-', 'people-card', 'team-editor')):
                x, y, w, h = n['rect']
                assert x >= -.5 and x+w <= shot['width']+.5, (path, n)
    for width in (1280, 640):
        for theme in ('light', 'dark'):
            ours = elements('people-our-team', width, theme)
            external = elements('people-external', width, theme)
            assert persons(ours) == {'person-alex chen', 'person-jamie park', 'person-morgan lee'}
            assert persons(external) == {'person-dana ortiz', 'person-leo park'}
            assert persons(elements('people-search', width, theme)) == {'person-alex chen'}
            assert not persons(elements('people-import', width, theme))
            assert any(n.get('text') == 'People import is not available yet' for n in elements('people-import', width, theme))
            assert any(n.get('text') == 'also Alex C' for n in ours)
            assert any(n.get('text') == 'People from older meetings appear when you meet them again.' for n in ours)
            assert len([n for n in elements('team-editor', width, theme) if n.get('key') == 'team-editor']) == 1
            editor_nodes = elements('team-editor', width, theme)
            assert keyed(editor_nodes, 'team-members')['role'] != 'textbox'
            assert not any(n.get('key') == 'team-member-search' for n in editor_nodes)
            assert keyed(editor_nodes, 'team-edit-0')['label'] == 'Close Design'
            assert keyed(editor_nodes, 'team-member-alex chen')['fill'] != keyed(editor_nodes, 'team-member-morgan lee')['fill']
            assert not any(n.get('key') == 'team-name' for n in ours)
    for page, expectations in [('people-our-team', [('alex chen', 'Design', 3, 3), ('jamie park', 'Design', 2, 0), ('morgan lee', 'Delivery', 2, 1)]), ('people-external', [('dana ortiz', 'No team', 3, 2), ('leo park', 'No team', 1, 0)])]:
        nodes = elements(page)
        for name, team, meetings, opened in expectations:
            row = keyed(nodes, 'person-'+name)['path']
            for column, value in [(1, team), (2, str(meetings)), (3, '11 Nov'), (4, str(opened))]:
                assert any(n.get('path') == f'{row}/#{column}' and n.get('text') == value for n in nodes), (name, column, value)
    print('PASS: 24 native renders, four pixel-identical General references, real roster/alias/search rows and horizontal bounds', flush=True)

    def shot(name, scene='people-our-team', events=None, state=None):
        result = render(exe, name, scene, events=events, state=state)
        return elements(name), Path(result['state'])

    nodes, state = shot('side-corrected', events=[tap('person-side-alex chen/segment1')])
    assert 'person-alex chen' not in persons(nodes)
    assert json.loads((state/'local/ui.json').read_text())['people_sides']['alex chen'] == 'external'
    nodes, _ = shot('side-restored', 'people-external', state=state)
    assert 'person-alex chen' in persons(nodes)
    nodes, _ = shot('side-reversed', 'people-external', [tap('person-side-alex chen/segment0')], state)
    assert 'person-alex chen' not in persons(nodes)
    nodes, _ = shot('side-back-on-team', state=state)
    assert 'person-alex chen' in persons(nodes)

    nodes, state = shot('tab-saved', events=[tap('settings-tab-1'), tap('settings-tab-2', .7)])
    assert json.loads((state/'local/ui.json').read_text())['settings_tabs']['People'] == 2
    nodes, _ = shot('tab-restored', 'settings-general', [tap('settings-back'), tap('profile-button', .7), tap('profile-menu-item4', 1.1), tap('settings-page-People', 1.5)], state)
    assert any(n.get('key') == 'people-import' for n in nodes)
    assert any(n.get('text') == 'Settings / People / Import' for n in nodes), 'Breadcrumb does not show Import'

    nodes, _ = shot('alias-search', events=[tap('people-search'), text('Alex C', .6)])
    assert persons(nodes) == {'person-alex chen'}
    nodes, _ = shot('search-escape', events=[tap('people-search'), text('Alex C', .6), key('Escape', 2)])
    assert len(persons(nodes)) == 3
    nodes, _ = shot('team-search', events=[tap('people-search'), text('Delivery', .6)])
    assert persons(nodes) == {'person-morgan lee'}

    nodes, state = shot('team-renamed', 'people-team-editor', [tap('team-name'), select_all(.6), text('Product', .9), tap('team-save', 3)])
    teams = json.loads((state/'teams.json').read_text())['teams']
    assert [t['name'] for t in teams] == ['Product', 'Delivery'], teams
    assert teams[0]['aliases'] == {'Alex C':'Alex Chen'}
    assert not any(n.get('key') == 'team-editor' for n in nodes)
    nodes, _ = shot('team-rename-restored', state=state)
    assert any(n.get('text') == 'Product' for n in nodes)
    nodes, state = shot('team-cancelled', 'people-team-editor', [tap('team-name'), select_all(.6), text('Discard', .9), tap('team-cancel', 3)])
    assert json.loads((state/'teams.json').read_text())['teams'][0]['name'] == 'Design'
    assert not any(n.get('key') == 'team-editor' for n in nodes)
    nodes, _ = shot('team-escape', 'people-team-editor', [tap('team-name'), key('Escape', .6)])
    assert not any(n.get('key') == 'team-editor' for n in nodes)
    assert any(n.get('key') == 'people-roster' for n in nodes)
    nodes, _ = shot('team-close', 'people-team-editor', [tap('team-edit-0')])
    assert not any(n.get('key') == 'team-editor' for n in nodes)
    assert keyed(nodes, 'team-edit-0')['label'] == 'Open Design'
    nodes, _ = shot('team-one-editor', 'people-team-editor', [tap('team-edit-1')])
    assert len([n for n in nodes if n.get('key') == 'team-editor']) == 1
    field = keyed(nodes, 'team-name')
    assert any(n.get('text') == 'Delivery' and n.get('path', '').startswith(field['path']+'/') for n in nodes)
    nodes, state = shot('team-duplicate', 'people-team-editor', [tap('team-name'), select_all(.6), text('Delivery', .9), tap('team-save', 3)])
    assert any(n.get('text') == 'Use a different name for each team.' for n in nodes)
    assert [t['name'] for t in json.loads((state/'teams.json').read_text())['teams']] == ['Design', 'Delivery']
    nodes, state = shot('team-added', events=[tap('team-add'), tap('team-name', .7), text('Support', 1), tap('team-member-morgan lee', 3), tap('team-advanced', 5), tap('team-aliases', 5.4), text('Morgan M = Morgan Lee', 5.7), scroll(7.3), tap('team-save', 8)])
    teams = json.loads((state/'teams.json').read_text())['teams']
    assert len(teams) == 3 and teams[2] == dict(name='Support', members=['Morgan Lee'], aliases={'Morgan M':'Morgan Lee'}), teams
    remove = [tap('team-advanced'), scroll(.6), tap('team-remove', 1)]
    nodes, state = shot('team-remove-prompt', 'people-team-editor', remove)
    assert any(n.get('key') == 'team-remove-confirm' for n in nodes)
    assert len(json.loads((state/'teams.json').read_text())['teams']) == 2
    nodes, state = shot('team-remove-cancel', 'people-team-editor', remove+[tap('team-keep', 1.5)])
    assert not any(n.get('key') == 'team-remove-confirm' for n in nodes)
    assert len(json.loads((state/'teams.json').read_text())['teams']) == 2
    nodes, state = shot('team-removed', 'people-team-editor', remove+[tap('team-remove-confirm', 1.5)])
    assert [t['name'] for t in json.loads((state/'teams.json').read_text())['teams']] == ['Delivery']

    state = Path(tempfile.mkdtemp(prefix='first-rollout-', dir=OUT/'state'))
    (state/'local').mkdir()
    (state/'local/ui.json').write_text(json.dumps(dict(tasks_mine=True)))
    nodes, _ = shot('old-roster-hidden', state=state)
    assert not persons(nodes)
    assert len([n for n in nodes if n.get('key', '').startswith('team-row-')]) == 2
    prefs = json.loads((state/'local/ui.json').read_text())
    assert len(prefs['people_existing_meetings']) == 3 and prefs['tasks_mine']
    folder = state/'people-meetings/1800000000003-1'
    folder.mkdir()
    (folder/'english-summary.json').write_text(json.dumps(dict(schema=2, title='New encounter', people=['Old Only'])))
    (folder/'manifest.json').write_text(json.dumps(dict(status='complete', created_at_ms=prefs['people_since_ms']+1000)))
    nodes, _ = shot('old-person-returns', 'people-external', state=state)
    assert persons(nodes) == {'person-old only'}
    assert any(n.get('text') == 'Side not set' for n in nodes)
    verify_member_search(exe)
    (OUT/'validation.json').write_text(json.dumps(dict(result='pass', matrix_renders=24, pixel_identical_general=4, checks=['eligibility and new encounter', 'aliases and aggregate counts', 'side correction and restart', 'tab persistence and breadcrumb', 'search and Escape', 'one editor', 'rename and restart', 'Add with aliases', 'Cancel and Escape', 'duplicate rejection', 'remove confirmation']), indent=2))
    print('PASS: native persistence, side corrections, remembered tabs, alias/team search, one team editor, rename/add/Cancel/Escape and old-only eligibility', flush=True)


if __name__ == '__main__':
    main()
