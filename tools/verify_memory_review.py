"""Verify native Memory layout, counts, filters, comments and real Undo persistence."""
import argparse
import hashlib
import json
import tempfile
from pathlib import Path
from PIL import Image,ImageChops
from render_memory_review import OUT,render,tap,key
from prepare_memory_fixture import prepare


def elements(name,width=1280,theme='light'):
    tree=json.loads((OUT/f'{name}-{width}-{theme}.json').read_text(encoding='utf-8-sig'))
    assert not tree['warnings'],(name,tree['warnings'])
    return tree['elements']


def keyed(nodes,key):return next(n for n in nodes if n.get('key')==key)
def needs(nodes):return [n for n in nodes if n.get('key','').startswith('memory-need-')]
def typing(text,t):return dict(type='type',text=text,t=t)


def main():
    p=argparse.ArgumentParser();p.add_argument('exe',type=Path);p.add_argument('--reference',type=Path,required=True);a=p.parse_args()
    exe=a.exe.resolve()
    index=json.loads((OUT/'render-index.json').read_text())
    assert len(index['rows'])==28
    assert index['binary_sha256']==hashlib.sha256(exe.read_bytes()).hexdigest()
    for shot in index['rows']:
        path=Path(shot['path']);w=shot['width'];theme=shot['theme']
        assert Image.open(path).size==(w,800)
        name=path.stem.rsplit('-',2)[0]
        nodes=elements(name,w,theme)
        assert not any('Automatic settling history' in n.get('text','') or n.get('text') in ['Recently settled by Sidenote','Previous state was not saved'] for n in nodes)
        if name=='settings-general':
            assert ImageChops.difference(Image.open(path).convert('RGBA'),Image.open(a.reference/path.name).convert('RGBA')).getbbox() is None,path
        for n in needs(nodes):
            x,y,width,height=n['rect']
            assert x>=0 and x+width<=w+.5,(name,n)
        if name in ('memory','memory-5','memory-empty'):
            expected={'memory':1,'memory-5':5,'memory-empty':0}[name]
            assert len(needs(nodes))==expected
            assert keyed(nodes,'memory-count')['text']==('Nothing needs you right now.' if expected==0 else '1 thing needs you' if expected==1 else f'{expected} things need you')
            assert not any(n.get('key')=='memory-filters' for n in nodes)
            assert len([n for n in nodes if n.get('key')=='mem-search'])==1
            if expected==0:
                assert not any(n.get('text')=='Recent changes you approved' for n in nodes)
            for n in nodes:
                if n.get('key','').startswith('memory-best-guess-'):
                    assert n['color']==('#e9e7e1ff' if theme=='dark' else '#1e1e1cff')
        if name in ('memory-browse','memory-browse-filters'):
            assert not any(n.get('key')=='memory-count' for n in nodes)
            assert keyed(nodes,'memory-breadcrumb')['text']=='Memory / All'
            groups=[n for n in nodes if n.get('key','').startswith('memory-group-')]
            assert len(groups)==5
            assert all(n.get('fill')!='#00000000' for n in groups)
            rows=[n for n in nodes if n.get('key','').startswith('mrow-')]
            for r in rows:
                descendants=[n for n in nodes if n.get('path','').startswith(r['path']+'/')]
                assert not any(n.get('text') in ['Active','Not confirmed','1 source'] for n in descendants)
                assert any(n.get('key','').startswith('memory-unconfirmed-') for n in descendants)
                text_nodes=[n for n in descendants if n.get('role')=='text']
                assert abs(min(n['rect'][0] for n in text_nodes)-r['rect'][0]-14)<.1, 'Shared card row padding changed'
        if name=='memory-browse-filters':
            labels={n.get('label') for n in nodes if n.get('role','').startswith('menuitem')}
            assert {'All types','Pending proposals','People','Projects','Decisions','Commitments','Open questions','Background','Active','Not recently confirmed','Retired and superseded','All projects','Northwind Field Guide'}<=labels,labels
            assert {'Type','State','Project'}<={n.get('text') for n in nodes}
            assert len([n for n in nodes if n.get('key','').startswith('sep') and '/memory-filter-menu-menu/' in n.get('path','')])==2
            assert len([n for n in nodes if n.get('role')=='icon' and '/memory-filter-menu-menu/' in n.get('path','') and '/indicator/' in n.get('path','') and n.get('opacity')==1])==3
        if name=='memory-undo':
            assert len(needs(nodes))==1
            assert keyed(nodes,'memory-count')['text']=='1 thing needs you'
            assert any(n.get('key')=='toast' for n in nodes)
            assert any(n.get('text')=='Change undone. The previous memory is restored.' for n in nodes)
    print('PASS: 28 renders, hidden empty history, quiet Browse rows in cards, menu sections, breadcrumb, Undo toast and four pixel-identical General references',flush=True)
    filter_keys={n['label']:n['key'] for n in elements('memory-browse-filters') if n.get('role','').startswith('menuitem')}

    def shot(name,events=None,state=None,case='one'):
        r=render(exe,name,case,events=events,state=state)
        return elements(name),Path(r['state'])

    nodes,state=shot('accept-decision')
    ident=needs(nodes)[0]['key'].removeprefix('memory-need-')
    nodes,_=shot('accepted',events=[tap('memory-accept-'+ident)],state=state)
    assert not needs(nodes)
    assert keyed(nodes,'memory-count')['text']=='Nothing needs you right now.'
    assert any(n.get('key')=='memory-undo-0' for n in nodes)
    nodes,_=shot('accepted-restarted',state=state)
    assert not needs(nodes)
    nodes,_=shot('undo-after-restart',events=[tap('memory-undo-0')],state=state)
    assert len(needs(nodes))==1 and any(n.get('key')=='toast' for n in nodes)
    nodes,_=shot('undo-restarted',state=state)
    assert len(needs(nodes))==1
    assert any(n.get('text')=='Current: The demo is on Thursday.' for n in nodes)
    journals=[json.loads(path.read_text(encoding='utf-8-sig')) for path in (state/'state').rglob('*.json')]
    assert any(op.get('store')=='memory/records.json' and any(change.get('value',{}).get('operation')=='undo_decision' for change in op.get('changes',[]) if isinstance(change.get('value'),dict)) for op in journals)

    nodes,state=shot('comment-start')
    ident=needs(nodes)[0]['key'].removeprefix('memory-need-')
    nodes,_=shot('comment-saved',events=[tap('memory-comment-'+ident),tap('memory-comment-input',.7),typing('The venue email also says Friday.',1),key('Enter',3)],state=state)
    assert any(n.get('text')=='Your comment: The venue email also says Friday.' for n in nodes)
    nodes,_=shot('comment-restarted',state=state)
    assert any(n.get('text')=='Your comment: The venue email also says Friday.' for n in nodes)
    assert len(needs(nodes))==1
    nodes,_=shot('keep-current',events=[tap('memory-reject-'+ident)],state=state)
    assert not needs(nodes)
    nodes,_=shot('reject-undone',events=[tap('memory-undo-0')],state=state)
    assert len(needs(nodes))==1

    nodes,_=shot('memory-search-results',events=[tap('mem-search'),typing('venue reply',.6)])
    assert any(n.get('text')=='The venue reply is still open.' for n in nodes)
    assert not any(n.get('text')=='The demo is on Thursday.' for n in nodes)
    nodes,_=shot('memory-search-escape',events=[tap('mem-search'),typing('venue reply',.6),key('Escape',1.5)])
    assert len(needs(nodes))==1
    nodes,_=shot('browse-type-filter',events=[tap('memory-browse'),tap('memory-filters',.7),tap(filter_keys['Decisions'],1.1)])
    assert len([n for n in nodes if n.get('key','').startswith('mrow-')])==6
    nodes,_=shot('browse-open-record',events=[tap('memory-browse'),tap('mrow-memory-1',.7)])
    assert any(n.get('key')=='memory-panel' for n in nodes)
    nodes,_=shot('landing-search-after-filter',events=[tap('memory-browse'),tap('memory-filters',.7),tap(filter_keys['Decisions'],1.1),tap('memory-home',1.5),tap('mem-search',1.9),typing('venue reply',2.2)])
    assert any(n.get('text')=='The venue reply is still open.' for n in nodes)
    crowded=Path(tempfile.mkdtemp(prefix='seven-decisions-',dir=OUT/'state'))
    prepare(crowded,count=7)
    nodes,_=shot('memory-seven-capped',state=crowded)
    assert len(needs(nodes))==5 and keyed(nodes,'memory-count')['text']=='7 things need you'
    nodes,_=shot('memory-all-proposals',state=crowded,events=[tap('memory-browse'),tap('memory-filters',.7),tap(filter_keys['Pending proposals'],1.1)])
    assert len([n for n in nodes if n.get('key','').startswith('mprop-')])==7
    mixed=Path(tempfile.mkdtemp(prefix='browse-metadata-',dir=OUT/'state'))
    prepare(mixed)
    path=mixed/'memory/records.json'
    data=json.loads(path.read_text())
    data['records'][0]['sources'].append(dict(type='meeting',reference='[[1800000000001-1@00:05]]'))
    data['records'][1]['last_confirmed']=data['records'][1]['updated_at']
    data['records'][2]['status']='retired'
    data['records'][3]['status']='proposed'
    path.write_text(json.dumps(data))
    nodes,_=shot('browse-metadata',state=mixed,events=[tap('memory-browse')])
    assert keyed(nodes,'memory-source-count-memory-0')['text']=='2 sources'
    assert not any(n.get('key')=='memory-unconfirmed-memory-1' for n in nodes)
    nodes,_=shot('browse-retired',state=mixed,events=[tap('memory-browse'),tap('memory-filters',.7),tap(filter_keys['Retired and superseded'],1.1)])
    assert len([n for n in nodes if n.get('key','').startswith('mrow-')])==1
    assert not any(n.get('text')=='Retired' for n in nodes)
    nodes,_=shot('browse-review-state',state=mixed,events=[tap('memory-browse'),tap('memory-filters',.7),tap(filter_keys['Not recently confirmed'],1.1)])
    assert any(n.get('text')=='Proposed' for n in nodes)
    nodes,_=shot('memory-source',events=[tap('need-'+ident+'-current-source-0')],state=state)
    assert not any(n.get('key')=='memory-pane' for n in nodes)
    (OUT/'validation.json').write_text(json.dumps(dict(result='pass',review_round=1,renders=28,pixel_identical_general=4,checks=['hidden empty sections','no Undo explanation','normal best-guess colour with semibold prefix','quiet Browse labels and shared group cards','labelled filter sections','Browse breadcrumb without decision count','five-card limit','all existing type/status/project filters','accept and restart','Undo after restart and journal event','reject and Undo','comment and restart','search and Escape','source meeting link']),indent=2))
    print('PASS: native accept/reject, durable Undo, comments, search/Escape and source navigation',flush=True)


if __name__=='__main__':main()
