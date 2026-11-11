"""Verify native meeting layout, real interactions and unchanged Settings pixels."""
import argparse,hashlib,json,tempfile
from pathlib import Path
from PIL import Image,ImageChops
from render_meeting_review import OUT,ROOT,ID,render,prepare,tap,key

def elements(name,width=1280,theme='light'):
    d=json.loads((OUT/f'{name}-{width}-{theme}.json').read_text(encoding='utf-8-sig'))
    assert not d['warnings'],(name,d['warnings'])
    return d['elements']
def keyed(nodes,key):return next(n for n in nodes if n.get('key')==key)
def typing(text,t):return dict(type='type',text=text,t=t)
def move(t=1.5):return dict(type='move',x=10,y=700,t=t)
def main():
    p=argparse.ArgumentParser();p.add_argument('exe',type=Path);p.add_argument('--reference',type=Path,required=True);a=p.parse_args();exe=a.exe.resolve()
    index=json.loads((OUT/'render-index.json').read_text())
    assert len(index['rows'])==28
    assert index['binary_sha256']==hashlib.sha256(exe.read_bytes()).hexdigest()
    for shot in index['rows']:
        path=Path(shot['path']);w=shot['width'];theme=shot['theme'];name=path.stem.rsplit('-',2)[0];nodes=elements(name,w,theme)
        assert Image.open(path).size==(w,800)
        if name=='settings-general':
            assert ImageChops.difference(Image.open(path).convert('RGB'),Image.open(a.reference/path.name).convert('RGB')).getbbox() is None,path
            continue
        assert keyed(nodes,'meeting-title')['rect'][3]<=80.1
        assert keyed(nodes,'meeting-breadcrumb')['text'].startswith('Meetings / ')
        assert not any(n.get('key')=='sec-decisions' for n in nodes)
        if w==1280:
            rail=keyed(nodes,'meeting-rail-scroll');main=keyed(nodes,'meeting-main')
            assert abs(rail['rect'][2]-324)<.1 and main['rect'][2]>600
            if name!='meeting-no-tasks':
                for i in range(4):
                    r=keyed(nodes,f'meeting-task-{i}')['rect'];assert r[1]>=192 and r[1]+r[3]<800
        else:
            assert not any(n.get('key')=='meeting-rail-scroll' for n in nodes)
            assert keyed(nodes,'meeting-actions')['rect'][1]>keyed(nodes,'topic-3')['rect'][1]
        if name=='meeting-no-tasks':
            assert not any(n.get('key') in ['meeting-tasks','meeting-questions'] for n in nodes)
        if name=='meeting-source-hover':assert keyed(nodes,'meeting-source-icon-pt-0-0')['opacity']==1
        elif name=='meeting':assert keyed(nodes,'meeting-source-icon-pt-0-0')['opacity']==0
        if name=='meeting-transcript-sheet':
            sheet=keyed(nodes,'transcript-sheet')['rect'];assert sheet[0]>=0 and sheet[0]+sheet[2]<=w
            ask=keyed(nodes,'ask-pill')['rect'];lead=keyed(nodes,'meeting-transcript')['rect']
            assert abs(sheet[1]+sheet[3]+10-min(ask[1],lead[1]))<1
            assert 400<=sheet[3]<=450
            assert keyed(nodes,'transcript-sheet')['fill'].endswith('ff')
            assert keyed(nodes,'transcript-panel')['fill'].endswith('ff')
            back=keyed(nodes,'back-to-summary');assert '/transcript-header/' in back['path']
            assert any(n.get('key')=='meeting-summary-dim' for n in nodes)
            assert any(n.get('key')=='transcript-close' for n in nodes)
            assert not any(n.get('text')=='Minimise' for n in nodes)
            selected=next(n for n in nodes if n.get('text','').startswith('Then let us launch in the second week'))
            assert selected['rect'][1]>=sheet[1] and selected['rect'][1]+selected['rect'][3]<=sheet[1]+sheet[3]
            assert any(n.get('text')=='29 lines' for n in nodes)
            assert not any(n.get('key')=='transcript-jump' for n in nodes)
        if name=='meeting-wide-ask' and w==1280:
            pill=keyed(nodes,'ask-pill')['rect'];lead=keyed(nodes,'meeting-transcript')['rect']
            baseline=keyed(elements('meeting',w,theme),'ask-pill')['rect']
            assert pill[2]>baseline[2]+40
            assert abs(lead[1]+lead[3]/2-pill[1]-pill[3]/2)<1
            assert lead[0]+lead[2]+8<=pill[0]
        assert not any(n.get('text') in ['Maya','Priya','Tom'] for n in nodes if '/meeting-point-' in n.get('path',''))
        assert any(n.get('text')=='Sidenote updated 1 thing from this meeting.' for n in nodes)
        assert keyed(nodes,'meeting-memory')['text']=='See in Memory'
    print('PASS: 28 renders, task visibility, rail placement, source highlight and four pixel-identical Settings renders',flush=True)
    def shot(name,events=None,state=None,width=1280):
        r=render(exe,name,width=width,events=events,state=state)
        return elements(name,width),Path(r['state'])
    nodes,_=shot('meeting-source-focus',[tap('meeting-template'),key('Escape',.7),key('Tab',1),key('Tab',1.3),move(1.6)])
    focused=[n for n in nodes if n.get('focused')];print('Focused source:',[(n.get('key'),n.get('label')) for n in focused],flush=True)
    assert any(n.get('key')=='pt-0-0' and n.get('focused') for n in nodes)
    nodes,_=shot('meeting-keyboard-source',[tap('meeting-template'),key('Escape',.7),key('Tab',1),key('Tab',1.3),key('Enter',1.6),move(2)])
    assert any(n.get('key')=='transcript-sheet' for n in nodes)
    nodes,_=shot('meeting-sheet-closed',[tap('meeting-transcript'),tap('transcript-close',.8),move()])
    assert not any(n.get('key')=='transcript-sheet' for n in nodes)
    assert any(n.get('key')=='meeting-transcript' and n.get('focused') for n in nodes)
    nodes,_=shot('meeting-transcript-search',[tap('meeting-transcript'),tap('transcript-filter',.7),typing('survey',1),move()])
    assert any('lines match' in n.get('text','') for n in nodes)
    assert not any(n.get('text','').startswith('Then let us launch') for n in nodes)
    nodes,_=shot('meeting-time-open',[tap('meeting-transcript'),tap('transcript-time-toggle',.7)])
    assert any(n.get('key')=='transcript-jump' for n in nodes)
    nodes,_=shot('meeting-memory-filter',[tap('meeting-memory')])
    assert any(n.get('text','').startswith('From Northwind onboarding') for n in nodes)
    assert any(n.get('key')=='mrow-meeting-memory-0' for n in nodes)
    assert not any(n.get('key')=='mrow-meeting-memory-1' for n in nodes)
    print('PASS: source keyboard focus, Enter, useful close focus, transcript search/time and meeting-filtered Memory',flush=True)
    more=f'more-meeting-{ID}-button'
    nodes,state=shot('meeting-more',[tap(more)])
    menus={n.get('label'):n['key'] for n in nodes if n.get('role','').startswith('menuitem')}
    assert {'Reading','Tabs','Edit summary prompt','Write an email','Copy transcript','Copy summary','Copy decisions','Copy tasks','Open meeting folder','Export this meeting (Markdown)'}<=set(menus)
    nodes,state=shot('meeting-tabs',[tap(more),tap(menus['Tabs'],.7)],state)
    assert not any(n.get('key')=='meeting-header' for n in nodes)
    assert any(n.get('key')=='tab-summary' for n in nodes)
    nodes,_=shot('meeting-tabs-restarted',state=state)
    assert any(n.get('key')=='tab-summary' for n in nodes)
    assert json.loads((state/'local/ui.json').read_text())['meeting_tabs'] is True
    nodes,_=shot('meeting-tabs-transcript',[tap('tab-transcript')],state)
    assert any(n.get('key')=='transcript-panel' for n in nodes)
    assert not any(n.get('key')=='transcript-sheet' for n in nodes)
    nodes,_=shot('meeting-reading-restored',[tap(more),tap(menus['Reading'],.7)],state)
    assert any(n.get('key')=='meeting-header' for n in nodes)
    assert json.loads((state/'local/ui.json').read_text())['meeting_tabs'] is False
    nodes,state=shot('meeting-template-choices',[tap('meeting-template')])
    choices={n.get('label'):n for n in nodes if n.get('role','').startswith('menuitem')}
    assert {'Standard','Your instructions','Edit summary prompt'}<=set(choices)
    # A real custom instruction entry makes the existing second choice available.
    instructions=state/'agent-instructions.json'
    instructions.write_text(json.dumps(dict(notes=dict(text='Keep the existing meeting summary brief.',based_on=1,edited_at='2026-11-10T10:00:00Z'))))
    summary=state/'meeting-fixture'/ID/'english-summary.json'
    before=summary.read_bytes()
    nodes,_=shot('meeting-template-custom',[tap('meeting-template'),tap(choices['Your instructions']['key'],.7)],state)
    assert json.loads((state/'local/ui.json').read_text())['meeting_summary_templates'][ID]=='custom'
    assert summary.read_bytes()==before
    nodes,_=shot('meeting-template-restarted',state=state)
    assert any(n.get('text')=='Summary: Your instructions' for n in nodes)
    nodes,_=shot('meeting-template-standard',[tap('meeting-template'),tap(choices['Standard']['key'],.7)],state)
    assert json.loads((state/'local/ui.json').read_text())['meeting_summary_templates'][ID]=='standard'
    assert summary.read_bytes()==before
    nodes,_=shot('meeting-edit-prompt',[tap(more),tap(menus['Edit summary prompt'],.7)])
    assert any(n.get('key')=='agent-instructions' for n in nodes)
    nodes,state=shot('meeting-task-tick',[tap('meeting-tick-0')])
    assert keyed(nodes,'meeting-tick-0')['label'].endswith(' open')
    nodes,_=shot('meeting-task-tick-restarted',state=state)
    assert keyed(nodes,'meeting-tick-0')['label'].endswith(' open')
    nodes,_=shot('meeting-task-untick',[tap('meeting-tick-0')],state)
    assert keyed(nodes,'meeting-tick-0')['label'].endswith(' done')
    print('PASS: Tabs and Reading persistence, existing template choices, prompt link and task tick persistence',flush=True)
    # Explicit side corrections use the same local preference the People switch writes.
    state=Path(tempfile.mkdtemp(prefix='external-owner-',dir=OUT/'state'));prepare(state,'finished')
    (state/'local').mkdir(exist_ok=True)
    (state/'local/ui.json').write_text(json.dumps(dict(people_sides={'tom':'external'})))
    nodes,_=shot('meeting-external-owner',state=state)
    assert keyed(nodes,'meeting-task-count')['text'].endswith('Ours 3')
    assert keyed(nodes,'meeting-external-task-count')['text'].endswith('They will 1')
    assert not keyed(nodes,'meeting-our-side')['text'].endswith('Tom')
    # An overflowing rail scrolls without moving the narrative.
    state=Path(tempfile.mkdtemp(prefix='many-meeting-tasks-',dir=OUT/'state'));meetings=prepare(state,'finished')
    path=meetings/ID/'english-summary.json';data=json.loads(path.read_text())
    for i in range(4,12):data['todo'].append(dict(ref=f't{i}',title=f'Follow up on customer session {i}',description='Keep the approved session notes.',task=f'Follow up on customer session {i}',owner='Maya',at=['04:10']))
    path.write_text(json.dumps(data))
    nodes,_=shot('meeting-rail-before',state=state);before=keyed(nodes,'meeting-task-0')['rect'][1];context=keyed(nodes,'sec-context')['rect']
    nodes,_=shot('meeting-rail-scrolled',[dict(type='scroll',key='meeting-rail-scroll',dy=-500,dur=.3,t=.3)],state)
    assert keyed(nodes,'meeting-task-0')['rect'][1]<before-100
    assert keyed(nodes,'sec-context')['rect']==context
    nodes,_=shot('meeting-source-back',[tap('meeting-source-pt-0-0'),tap('back-to-summary',.9),move()])
    assert not any(n.get('key')=='transcript-sheet' for n in nodes)
    print('PASS: explicit external owners, independent rail scroll and return from a source',flush=True)
    nodes,_=shot('meeting-escape-search',[tap('meeting-transcript'),tap('transcript-filter',.7),typing('survey',1),key('Escape',1.4)])
    assert not any(n.get('key')=='transcript-sheet' for n in nodes)
    assert keyed(nodes,'meeting-transcript')['focused']
    nodes,_=shot('meeting-escape-time',[tap('meeting-transcript'),tap('transcript-time-toggle',.6),tap('transcript-jump',.9),key('Escape',1.2)])
    assert not any(n.get('key')=='transcript-sheet' for n in nodes)
    assert keyed(nodes,'meeting-transcript')['focused']
    nodes,_=shot('meeting-escape-ask',[tap('meeting-transcript'),tap('ask-input',.6),typing('Keep this draft question',.9),key('Escape',1.2)])
    assert not any(n.get('key')=='transcript-sheet' for n in nodes)
    assert any(n.get('text')=='Keep this draft question' for n in nodes)
    nodes,_=shot('meeting-source-closed',[tap('meeting-source-pt-0-0'),tap('transcript-close',.8),move()])
    assert not any(n.get('key') in ['transcript-sheet','back-to-summary'] for n in nodes)
    assert any(n.get('key')=='meeting-transcript' and n.get('focused') for n in nodes)
    nodes,_=shot('meeting-sheet-grown',[tap('meeting-transcript'),dict(type='drag',key='transcript-resize',dy=-60,dur=.3,t=.8)])
    height=keyed(nodes,'transcript-sheet')['rect'][3];assert 495<=height<=505
    assert abs(keyed(nodes,'transcript-sheet')['rect'][1]+height+10-keyed(nodes,'ask-pill')['rect'][1])<1
    nodes,_=shot('meeting-sheet-shrunk',[tap('meeting-transcript'),dict(type='drag',key='transcript-resize',dy=90,dur=.3,t=.8)])
    assert 345<=keyed(nodes,'transcript-sheet')['rect'][3]<=355
    nodes,_=shot('meeting-sheet-keyboard-resize',[tap('meeting-transcript'),tap('transcript-resize',.7),key('Down',1)])
    assert abs(keyed(nodes,'transcript-sheet')['rect'][3]-416)<1
    nodes,_=shot('meeting-sheet-minimum',[tap('meeting-transcript'),dict(type='drag',key='transcript-resize',dy=500,dur=.3,t=.8)])
    assert keyed(nodes,'transcript-sheet')['rect'][3]==180
    print('PASS: single Escape, source close focus, mouse resizing, keyboard resizing and minimum height',flush=True)
    for width in [1280,640]:
        render(exe,'meeting-wide-ask-sheet',case='wide-ask',width=width,events=[tap('meeting-transcript')])
        nodes=elements('meeting-wide-ask-sheet',width)
        pill=keyed(nodes,'ask-pill')['rect'];lead=keyed(nodes,'meeting-transcript')['rect'];sheet=keyed(nodes,'transcript-sheet')['rect']
        assert abs(sheet[1]+sheet[3]+10-min(pill[1],lead[1]))<1
        assert keyed(nodes,'ask-input')['rect'][2]>150
        assert pill[0]+pill[2]<=width and lead[0]>=0
        if width==1280:assert abs(lead[1]+lead[3]/2-pill[1]-pill[3]/2)<1
    print('PASS: wider Ask bar and longer model control keep the Transcript button and sheet aligned',flush=True)




if __name__=='__main__':main()
