"""Verify company render bounds, native navigation, edits and durable Undo offscreen."""
from pathlib import Path
import argparse
import json
from PIL import Image
from render_companies import OUT, fixture, render


def tap(key, t=.3):
    return dict(type='tap', key=key, t=t)


def nodes(shot):
    return json.loads(Path(shot['tree']).read_text(encoding='utf-8-sig'))['elements']


def texts(elements):
    return '\n'.join(n.get('text', '') for n in elements)


def has(elements, key):
    return any(n.get('key') == key for n in elements)


def main():
    p = argparse.ArgumentParser()
    p.add_argument('exe', type=Path)
    a = p.parse_args()
    exe = a.exe.resolve()
    index = json.loads((OUT / 'render-index.json').read_text())
    assert len(index['rows']) == 40
    for shot in index['rows']:
        assert Image.open(shot['path']).size == (shot['width'], 800)
        elements = nodes(shot)
        assert not has(elements, 'person-old only')
        if shot['scene'] != 'company-external':
            assert has(elements, 'company-tabs')
            for tab in ['Overview', 'People', 'Meetings', 'Decisions']:
                assert tab in texts(elements), (shot['path'], tab)
        if shot['scene'] == 'company-people':
            assert has(elements, 'person-dana ortiz') and has(elements, 'person-leo park')
            assert not has(elements, 'person-company-cell-dana ortiz')
            assert not any(n.get('text') == 'Company' for n in elements)
        assert 'Company and email' not in texts(elements)
        assert not any(n.get('key', '').startswith('person-company-link-') for n in elements)
        assert 'A status appears after the next meeting summary.' not in texts(elements)
        if shot['scene'] == 'company-decisions':
            assert len([n for n in elements if n.get('text') == 'Use the small prototype for the next workshop.']) == 1
            assert 'also in 2 earlier meetings' in texts(elements)
        if shot['scene'] != 'company-external':
            keyed = {n.get('key'): n for n in elements if n.get('key')}
            assert keyed['new-meeting']['label'] == 'New meeting', (shot['path'], keyed['new-meeting'])
            assert abs(keyed['company-title']['rect'][0] - keyed['company-back']['rect'][0]) < .5
            ask, shortcuts = keyed['ask-pill']['rect'], keyed['company-shortcuts']['rect']
            assert abs(ask[0] - shortcuts[0]) < .5 and abs(ask[2] - shortcuts[2]) < .5, (shot['path'], ask, shortcuts)
        if shot['scene'] == 'company-external':
            assert has(elements, 'company-header-fixture-northwind')
            assert has(elements, 'person-company-cell-dana ortiz')
            assert any(n.get('text') == 'All companies' for n in elements)
        if shot['scene'] == 'company-merge':
            assert has(elements, 'company-merge-dialog')
        if shot['scene'].startswith('company-empty-'):
            assert not any(n.get('key', '').startswith(('company-meeting-', 'company-decision-', 'person-dana')) for n in elements)
        for n in elements:
            if n.get('key', '').startswith(('company-tabs', 'company-task-', 'company-merge-dialog', 'person-dana', 'person-leo', 'person-side-', 'person-more-', 'person-company-cell-', 'company-header-')):
                x, _, w, _ = n['rect']
                assert x >= -.5 and x+w <= shot['width']+.5, (shot['path'], n.get('key'), n['rect'])
    print('PASS: 40 PNG sizes, company tabs, populated/empty data, hidden old people and horizontal bounds', flush=True)

    state, _ = fixture()
    shot = render(exe, 'check-header', 'company-external', state=state, events=[tap('company-header-fixture-northwind')])
    assert has(nodes(shot), 'company-tabs')
    shot = render(exe, 'check-tabs', 'company-saved', state=state, events=[tap('company-tabs/segment3')])
    assert has(nodes(shot), 'company-decision-0')
    shot = render(exe, 'check-source', 'company-decisions', state=state, events=[tap('company-decision-source-0')])
    assert not has(nodes(shot), 'company-tabs')
    shot = render(exe, 'check-decision-citations', 'company-decisions', state=state, events=[tap('company-decision-0')])
    assert all(has(nodes(shot), f'company-decision-citation-0-{i}') for i in range(3))
    assert len([n for n in nodes(shot) if n.get('text') == 'Use the small prototype for the next workshop.']) == 1
    shot = render(exe, 'check-decision-citation-source', 'company-decisions', state=state, events=[tap('company-decision-0'), tap('company-decision-citation-0-2', .8)])
    assert not has(nodes(shot), 'company-tabs')
    print('PASS: External header, native tabs, grouped decisions, expanded citations and source navigation', flush=True)

    rename = [tap('company-more'), tap('company-name', .6), dict(type='key', name='a', cmd=True, t=.8), dict(type='type', text='Northwind Labs', t=1), tap('company-save', 2.3)]
    render(exe, 'check-rename', 'company-saved', state=state, events=rename)
    shot = render(exe, 'check-rename-reloaded', 'company-saved', state=state)
    assert 'Northwind Labs' in texts(nodes(shot))
    shot = render(exe, 'check-rename-undo', 'company-saved', state=state, events=[tap('company-undo')])
    assert 'Northwind Labs' not in texts(nodes(shot))
    shot = render(exe, 'check-rename-undo-reloaded', 'company-saved', state=state)
    assert 'Northwind Labs' not in texts(nodes(shot))
    print('PASS: native rename and Undo survive independent app reloads', flush=True)

    shot = render(exe, 'check-merge', 'company-merge', state=state, events=[tap('company-merge-target-fixture-lumen'), tap('company-merge-confirm', .8)])
    assert 'Lumen' in texts(nodes(shot)) and not has(nodes(shot), 'company-merge-dialog')
    assert has(nodes(shot), 'company-undo')
    shot = render(exe, 'check-merge-undo', 'company-saved', state=state, events=[tap('company-undo')])
    assert 'Northwind' in texts(nodes(shot))
    shot = render(exe, 'check-merge-undo-reloaded', 'company-saved', state=state, events=[tap('company-tabs/segment1')])
    assert has(nodes(shot), 'person-dana ortiz') and has(nodes(shot), 'person-leo park')
    print('PASS: native merge and Undo restore company records and person links on reload', flush=True)

    shot = render(exe, 'check-finder', 'company-external', state=state, events=[dict(type='key', name='k', cmd=True, t=.3), dict(type='type', text='Northwind', t=.7), dict(type='key', name='Enter', t=1.7)])
    assert has(nodes(shot), 'company-tabs') and not any(n.get('key', '').startswith('finder-') for n in nodes(shot))
    print('PASS: global search opens company records', flush=True)
    shot = render(exe, 'check-company-cell', 'company-external', state=state, events=[tap('person-company-cell-dana ortiz')])
    assert has(nodes(shot), 'person-company-editor')
    shot = render(exe, 'check-all-companies', 'company-external', state=state, events=[tap('people-companies')])
    assert has(nodes(shot), 'company-add') and not has(nodes(shot), 'company-tabs')
    shot = render(exe, 'check-email', 'company-saved', state=state, events=[tap('company-tabs/segment1'), tap('person-more-dana ortiz', .7), tap('person-menu-dana ortiz-item0', 1.1), tap('person-email', 1.5), dict(type='type', text='dana@example.test', t=1.8), tap('person-company-save', 3.5)])
    assert has(nodes(shot), 'person-email-value-dana ortiz')
    shot = render(exe, 'check-email-reloaded', 'company-saved', state=state, events=[tap('company-tabs/segment1')])
    assert 'dana@example.test' in texts(nodes(shot))
    shot = render(exe, 'check-person-company', 'company-saved', state=state, events=[tap('company-tabs/segment1'), tap('person-more-dana ortiz', .7), tap('person-menu-dana ortiz-item1', 1.1), tap('person-company-fixture-lumen', 1.5), tap('person-company-save', 2)])
    assert not has(nodes(shot), 'person-dana ortiz') and has(nodes(shot), 'person-leo park')
    shot = render(exe, 'check-person-company-reloaded', 'company-saved', state=state, events=[tap('company-tabs/segment1')])
    assert not has(nodes(shot), 'person-dana ortiz')
    print('PASS: optional typed email and person company picker persist across reloads', flush=True)
    shot = render(exe, 'check-narrow-settings', 'company-external', width=390, events=[tap('settings-navigation'), tap('settings-page-General', .7)])
    assert 'Settings / General' in texts(nodes(shot))
    assert not has(nodes(shot), 'person-dana ortiz')
    print('PASS: narrow Settings navigation keeps the other pages reachable', flush=True)
    (OUT / 'interaction-results.json').write_text(json.dumps(dict(passed=True, checks=['40 render cases', 'header navigation', 'tabs', 'decision source', 'grouped decisions and expanded citations', 'company cell and All companies', 'Ask and title alignment', 'primary recording action', 'rename reload', 'rename undo reload', 'merge', 'merge undo person links reload', 'global search', 'typed email reload', 'person company picker reload', 'narrow Settings navigation']), indent=2))


if __name__ == '__main__':
    main()
