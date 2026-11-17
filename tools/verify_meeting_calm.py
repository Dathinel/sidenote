"""Verify finished-meeting templates with invented data through the native renderer.

Run against the baseline to record the red cases, then the changed binary for green.
"""
import argparse
import json
import tempfile
from pathlib import Path

import render_meeting_review as meeting
import render_people_review as native

ID = meeting.ID
FRAGMENTS = [(0, 'You', 'Okay'), (3, 'You', 'what data'), (5, 'You', 'okay'),
             (6, 'You', 'you need there'), (8, 'You', 'Age'),
             (21, 'Maya', 'Sure, I can pull that'), (24, 'Maya', 'from the export'),
             (80, 'You', 'Good')]


def fixture(root, lines=True):
    folder = root / 'meetings' / ID
    folder.mkdir(parents=True, exist_ok=True)
    (folder / 'manifest.json').write_text(json.dumps(dict(
        schema_version=1, title='Meeting at 15:14', title_source='default', status='complete',
        created_at_ms=1800000000000, duration_ms=120000)), encoding='utf-8')
    if lines:
        (folder / 'english-transcript.md').write_text('# Full English transcript\n\n' + ''.join(
            f'[00:{t // 60:02d}:{t % 60:02d}.000] {who} (English): {words}\n'
            for t, who, words in FRAGMENTS), encoding='utf-8')
    return root / 'meetings'


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('exe', type=Path)
    parser.add_argument('--out', type=Path, required=True)
    parser.add_argument('--record-only', action='store_true')
    args = parser.parse_args()
    exe, out = args.exe.resolve(), args.out.resolve()
    out.mkdir(parents=True, exist_ok=True)
    native.OUT = meeting.OUT = out
    rows, failures, checks = [], [], []

    def check(condition, name):
        checks.append(name)
        if not condition:
            failures.append(name)
            print('FAIL: ' + name, flush=True)

    def render(name, scene='meeting-reading-review', width=1280, events=None, state=None, meetings=None):
        row = native.render(exe, name, scene, width, 'dark', events, state, meetings)
        rows.append(row)
        tree = json.loads(Path(row['tree']).read_text(encoding='utf-8-sig'))
        check(not tree['warnings'], name + ': no renderer warnings')
        nodes = tree['elements']
        return nodes, {n['key']: n for n in nodes if n.get('key')}

    def finished(name, width=1280, events=None, case='finished', state=None):
        state = state or Path(tempfile.mkdtemp(prefix=name+'-', dir=out))
        source = meeting.prepare(state, case)
        return render(name, width=width, events=events, state=state, meetings=source)

    def texts(nodes):
        return [n['text'] for n in nodes if n.get('text')]

    for width in (1280, 390):
        name = f'finished-{width}'
        nodes, k = finished(name, width)
        check('meeting-template' not in k and 'meeting-our-side' not in k, name+': quiet header')
        check('meeting-view' in k and 'meeting-facts' in k, name+': toggle and plain facts')
        check('ask-model' not in k and 'ask-thinking' not in k, name+': no model pickers')
        check('meeting-recording-control' in k, name+': finished recording control')
        if 'meeting-transcript' in k:
            check(k['meeting-transcript'].get('label') == 'Show transcript', name+': labelled transcript control')
        nodes, k = finished(f'transcript-open-{width}', width, [native.tap('meeting-transcript')])
        check('meeting-transcript-view' in k and 'ask-input' not in k, name+': transcript replaces Ask')
        check('transcript-close' not in k and 'transcript-resize' not in k and 'meeting-summary-dim' not in k,
              name+': no detached sheet controls')
        if 'meeting-transcript-view' in k:
            x,y,w,h=k['meeting-transcript-view']['rect']
            check(0<=x and x+w<=width+1 and y>=0 and y+h<=800, name+': transcript fits')
            check(h>=250, name+': transcript keeps a readable height')
        finished(f'long-title-{width}', width, case='long-title')
        state = Path(tempfile.mkdtemp(prefix=f'fragments-{width}-', dir=out))
        source = fixture(state)
        nodes,k=render(f'summary-writing-{width}', 'notes-meeting-running', width, state=state, meetings=source)
        check('0 people' not in k.get('meeting-facts',{}).get('text',''), name+': unknown participants omitted')
        check('Writing notes' in texts(nodes), name+': inline progress')
        check(not any('15 minutes' in t or 'minutes ago' in t or 'Writing summary with' in t for t in texts(nodes)),
              name+': no engine or timer in progress')
        check(not any('updated 0' in t for t in texts(nodes)), name+': no zero Memory state')
        render(f'summary-done-{width}', 'notes-meeting-done', width)
        nodes,k=render(f'transcript-fragments-{width}', 'meeting-reading-review', width,
                       [native.tap('meeting-transcript')], state, source)
        check(any(t.startswith('Okay what data okay you need there Age') for t in texts(nodes)), name+': fragments grouped')
        check(any('Sure, I can pull that from the export' in t for t in texts(nodes)), name+': second speaker grouped')
        check('8 lines' in texts(nodes), name+': count preserves source lines')
        state=Path(tempfile.mkdtemp(prefix=f'empty-{width}-',dir=out));source=fixture(state,False)
        nodes,k=render(f'transcript-empty-{width}', 'meeting-reading-review', width,
                       [native.tap('meeting-transcript')],state,source)
        check('No transcript for this meeting.' in texts(nodes), name+': empty transcript')

    finished('finished-640',640)
    nodes,k=finished('ask-focused',events=[native.tap('ask-input',.4),dict(type='type',text='Draft question',t=.7)])
    check('ask-model' not in k and 'ask-thinking' not in k, 'focused Ask: pickers stay in Settings')
    check('ask-suggestions' not in k, 'focused Ask: no chips over the notes')
    nodes,k=finished('more',events=[native.tap(f'more-meeting-{ID}-button')])
    menus={n.get('label'):n['key'] for n in nodes if n.get('role','').startswith('menuitem')}
    needed={'Standard','Your instructions','Edit summary prompt','Reading','Tabs','Copy transcript',
            'Project','Memory from this meeting','Ask model and effort'}
    check(needed<=set(menus),'More keeps all moved features reachable')
    if 'Ask model and effort' in menus:
        nodes,k=finished('ask-settings',events=[native.tap(f'more-meeting-{ID}-button'),native.tap(menus['Ask model and effort'],.7)])
        check('ask-agent-row' in k and 'ask-settings-model-trigger' in k,'Ask settings: model and effort destination')

    for kind, stamp in [('citation','01:02.520'),('missing-citation','0:01')]:
        state=Path(tempfile.mkdtemp(prefix=kind+'-',dir=out));source=fixture(state)
        folder=source/ID
        (folder/'english-transcript.md').write_text('[00:01:00.000] You (English): Prepare the workshop samples.\n[00:01:03.000] Maya (English): I will send them.\n')
        (folder/'english-summary.json').write_text(json.dumps(dict(title='Workshop launch plan',
            topics=[dict(title='Workshop',points=[dict(text='Prepare workshop samples.',at=[stamp],type='decided')])])),encoding='utf-8')
        events=[native.tap('meeting-source-pt-0-0')]
        nodes,k=render(kind,events=events,state=state,meetings=source)
        if kind=='citation':
            check('meeting-transcript-view' in k,'fractional citation opens transcript')
            check(not any('no line for that time' in t for t in texts(nodes)),'fractional citation: no failure banner')
            if 'line-0' in k and 'meeting-transcript-view' in k:
                x,y,w,h=k['line-0']['rect'];px,py,pw,ph=k['meeting-transcript-view']['rect']
                check(y>=py and y+h<=py+ph,'fractional citation scrolls target into view')
            check(any(n.get('key')=='meeting-source-pt-0-0' and n.get('label')=='Open source at 1:02' for n in nodes),
                  'fractional citation uses an m:ss chip')
        else:
            nodes,k=render('notice-after-memory',events=events+[native.tap('view-memory',.8)],state=state,meetings=source)
            check(not any('no line for that time' in t for t in texts(nodes)),'page notice clears on Memory navigation')

    state=Path(tempfile.mkdtemp(prefix='preview-title-',dir=out));source=fixture(state)
    drafts=state/'local/draft-summaries';drafts.mkdir(parents=True)
    (drafts/(ID+'.json')).write_text(json.dumps(dict(title='Workshop launch plan',context=['A synthetic preview.'])))
    nodes,k=render('preview-title',state=state,meetings=source)
    check(k.get('meeting-title',{}).get('text')=='Workshop launch plan','preview summary title becomes the page heading')
    nodes,k=render('engine-collapsed','settings-engine',events=[native.tap('engine-unused',.3)] if not args.record_only else None)
    if not args.record_only:
        check('engine-unused' in k and 'engine-worker-threads-input' not in k,'Engine: unused controls collapsed')
        check('engine-notes-writer-change' in k,'Engine: active writer remains visible')
    render('engine-expanded','settings-engine')
    if not args.record_only:
        nodes,k=render('engine-checking','settings-engine-checking')
        check('settings-hex-check' in k,'Engine: status check is visible')
        check(not any('Try New meeting' in n.get('text','') for n in nodes if '/hex-pane/' in n.get('path','')),
              'Engine: no warning before check completes')
        nodes,k=render('engine-failed-check','settings-engine-failed-check')
        check(any('Could not confirm Hex recording status. Checking again automatically.' in t for t in texts(nodes)),
              'Engine: failed bounded check explains retry')
    (out/'index.json').write_text(json.dumps(dict(rows=rows,checks=checks,failures=failures),indent=2))
    print(f'{len(checks)-len(failures)}/{len(checks)} render checks passed; {len(rows)} scenes',flush=True)
    if failures and not args.record_only:
        raise SystemExit(1)


if __name__=='__main__':main()
