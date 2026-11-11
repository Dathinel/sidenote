"""Offscreen notebook keyboard, drag, local editing and approval-entry proofs."""
from pathlib import Path
import json,os,subprocess,sys,tempfile
repo=Path(__file__).resolve().parents[1];exe=Path(sys.argv[1]).resolve();out=Path(sys.argv[2]).resolve();out.mkdir(parents=True,exist_ok=True)
parent=repo/'target/nbe';parent.mkdir(parents=True,exist_ok=True)
def run(name,scene='notebook',events=None):
 state=Path(tempfile.mkdtemp(prefix=name+'-',dir=parent));
 for folder in ['appdata','localappdata','peers']:(state/folder).mkdir()
 (state/'hex-fixture.json').write_bytes((repo/'tools/ui-fixtures/hex.json').read_bytes())
 env=dict(os.environ,APPDATA=str(state/'appdata'),LOCALAPPDATA=str(state/'localappdata'),SIDENOTE_STATE_DIR=str(state))
 args=[str(exe),'--data-dir',str(state),'--peers-dir',str(state/'peers'),'--meetings-dir',str(repo/'tools/ui-fixtures/meetings'),'--scene',scene,'--snap',str(out/f'{name}.png'),'--tree',str(out/f'{name}.json'),'--size','1280x800','--theme','light']
 if events:
  script=out/f'{name}-script.json';script.write_text(json.dumps({'events':events},indent=2),encoding='utf-8');args+=['--script',str(script),'--at','mark:end']
 with (out/f'{name}.log').open('w',encoding='utf-8') as log:subprocess.run(args,cwd=repo,env=env,stdout=log,stderr=log,check=True,timeout=45,creationflags=subprocess.CREATE_NO_WINDOW)
 tree=json.loads((out/f'{name}.json').read_text(encoding='utf-8-sig'));assert not tree['warnings'],tree['warnings']
 books=[json.loads(p.read_text(encoding='utf-8')) for p in (state/'annotations/notebooks').glob('*.json')];assert len(books)==1
 assert not (state/'annotations/notebooks/jobs').exists(),'A headless UI action started a model job'
 return tree,books[0],state
base,_,_=run('notebook-base')
cards=sorted([n for n in base['elements'] if n.get('key','').startswith('notebook-capture-')],key=lambda n:n['rect'][1]);assert len(cards)==3
centers=[(r['rect'][0]+r['rect'][2]*.7,r['rect'][1]+r['rect'][3]*.5) for r in cards]
original=['Workshop setup','Materials for the prototype','Friday review'];results=[]
for name,index,key,expected in [('notebook-keyboard-down',0,'Down',[original[1],original[0],original[2]]),('notebook-keyboard-up',1,'Up',[original[1],original[0],original[2]])]:
 x,y=centers[index];tree,book,_=run(name,events=[{'t':.3,'type':'tap','x':x,'y':y},{'t':.6,'type':'key','name':key,'alt':True},{'t':1.4,'type':'mark','name':'end'}]);assert [r['title'] for r in book['captures']]==expected,(name,[r['title'] for r in book['captures']]);assert [s['capture_key'] for s in book['note']['value']['sections']]==[r['key'] for r in book['captures']];assert book['revision']==book['note_revision'];results.append({'case':name,'order':expected,'saved':True,'note_order_matches':True})
x,y=centers[0];dy=centers[2][1]-y
_,book,_=run('notebook-drag',events=[{'t':.3,'type':'drag','x':x,'y':y,'dx':0,'dy':dy,'dur':.4},{'t':1.5,'type':'mark','name':'end'}]);expected=[original[1],original[2],original[0]];assert [r['title'] for r in book['captures']]==expected;assert [s['capture_key'] for s in book['note']['value']['sections']]==[r['key'] for r in book['captures']];results.append({'case':'drag','order':expected,'saved':True})
_,book,_=run('notebook-edit',scene='notebook-note',events=[{'t':.3,'type':'tap','key':'edit-notebook-note'},{'t':.7,'type':'tap','key':'notebook-edit-/title'},{'t':1,'type':'key','name':'a','cmd':True},{'t':1.3,'type':'type','text':'Edited notebook note'},{'t':4,'type':'tap','key':'edit-notebook-note'},{'t':4.8,'type':'mark','name':'end'}]);assert book['note']['value']['title']=='Edited notebook note';results.append({'case':'edit','title':book['note']['value']['title'],'saved':True})
tree,book,state=run('notebook-review-required',scene='notebook-needs-review',events=[{'t':.3,'type':'tap','key':'write-notebook-note'},{'t':2.8,'type':'mark','name':'end'}]);keys={n.get('key') for n in tree['elements']};assert 'annotation-send' in keys and any(k and k.startswith('review-image-') for k in keys),'Missing capture did not enter image review';results.append({'case':'missing note','review_required':True,'no_model_job':True})
(out/'workflow-proof.json').write_text(json.dumps(results,indent=2),encoding='utf-8');print('PASS: Alt+Down, Alt+Up, drag, saved note edits and mandatory capture review; no headless model jobs.')
