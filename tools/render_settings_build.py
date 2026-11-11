"""Render Settings in the isolated offscreen application, never on the desktop."""
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path
import argparse,json,os,subprocess,sys,shutil
ROOT=Path(__file__).resolve().parents[1]
OUT=ROOT/'artifacts/settings-build'
STATE=ROOT/'target/settings-build-render'
EXE=ROOT/'target/x86_64-pc-windows-gnu/debug/sidenote.exe'
sys.path.insert(0,str(ROOT/'tools'))
from prepare_people_fixture import prepare
PAGES=[('general','settings-general',900),('meetings-recording','settings-recording',2200),('meetings-summaries','settings-summaries',1200),('meetings-tasks','settings-tasks',900),('prompt-library','settings-prompts',1800),('people','settings-people',1800),('people-external','people-external',1800),('hex-dictation','settings-hex',1800),('models-agents','settings-helpers',1800),('models-speech','settings-speech',1400),('appearance','settings-appearance',900),('sync','settings-sync',900),('privacy-data','settings-privacy',1800),('updates','settings-updates',900),('diagnostics','settings-diagnostics',1800)]

def render(job):
 name,scene,width,theme,height,events=job
 base=OUT/f'settings-{name}-{width}-{theme}'
 state=STATE/base.name
 for d in ('appdata','localappdata','peers'):(state/d).mkdir(parents=True,exist_ok=True)
 fixture=json.loads((ROOT/'tools/ui-fixtures/hex.json').read_text(encoding='utf-8'))
 fixture['settings']+=json.loads((ROOT/'tools/ui-fixtures/settings-recording.json').read_text(encoding='utf-8'))
 (state/'hex-fixture.json').write_text(json.dumps(fixture),encoding='utf-8')
 (state/'config.json').write_text(json.dumps(dict(version=1,features=dict(tasks=True,memory=True,daily_check=True,projects=True))),encoding='utf-8')
 (state/'projects.json').write_text(json.dumps(dict(projects=['Northwind Atlas','Northwind Field Guide'],aliases={})),encoding='utf-8')
 meetings=prepare(state)
 if scene=='memory':
  from prepare_memory_fixture import prepare as prepare_memory
  meetings=prepare_memory(state)
 if scene.startswith('screens-'):
  meetings=state/'screen-meetings'
  shutil.copytree(ROOT/'tools/ui-fixtures/screens/meetings',meetings,dirs_exist_ok=True)
  summary=meetings/'1793955600000-1/english-summary.json'
  value=json.loads(summary.read_text(encoding='utf-8'));value['title']='Planning the next Northwind workshop, partner review, delivery timeline and follow-up decisions for the whole team'
  summary.write_text(json.dumps(value),encoding='utf-8')
 env=os.environ|{'PATH':r'E:\Personal-Projects\tools\hex-meetings-gnu\w64devkit\bin;'+os.environ['PATH'],'APPDATA':str(state/'appdata'),'LOCALAPPDATA':str(state/'localappdata'),'SIDENOTE_STATE_DIR':str(state)}
 args=[str(EXE),'--meetings-dir',str(meetings),'--data-dir',str(state),'--peers-dir',str(state/'peers'),'--scene',scene,'--theme',theme,'--size',f'{width}x{height}','--scale','1','--snap',str(base)+'.png','--tree',str(base)+'.json']
 if events:
  script=state/'events.json';script.write_text(json.dumps(dict(events=events+[dict(t=max(e['t'] for e in events)+1,type='mark',name='end')])),encoding='utf-8')
  args+=['--script',str(script),'--at','mark:end']
 with (state/'render.log').open('w') as log:
  result=subprocess.run(args,cwd=ROOT,env=env,stdout=log,stderr=log,creationflags=subprocess.CREATE_NO_WINDOW,timeout=180)
 assert result.returncode==0,(base.name,(state/'render.log').read_text()[-2000:])
 assert base.with_suffix('.png').is_file(),base
 tree=json.loads(base.with_suffix('.json').read_text(encoding='utf-8-sig'))
 assert not tree['warnings'],(base.name,tree['warnings'])
 print(base.name,flush=True)
 return str(base.with_suffix('.png').relative_to(ROOT))

def tap(key,t=.3):return dict(type='tap',key=key,t=t)
def main():
 parser=argparse.ArgumentParser();parser.add_argument('--quick',action='store_true');parser.add_argument('--only');parser.add_argument('--search-keys',action='store_true');args=parser.parse_args()
 OUT.mkdir(parents=True,exist_ok=True)
 jobs=[]
 setup_scenes=[('hex-setup','settings-hex-setup'),('hex-installing','settings-hex-installing'),('hex-install-failed','settings-hex-install-failed'),('hex-installed','settings-hex-installed')]
 for theme in ('dark','light'):
  for width in (1280,390):
   for name,scene in setup_scenes:
    jobs.append((name,scene,width,theme,900,None))
   for name,scene,height in PAGES:
    jobs.append((name,scene,width,theme,height,None))
   for name,scene,events in [('updates-available','updates-ready',None),('updates-checking','updates-checking',None),('hex-scrolled','settings-hex-expanded',[tap('settings-jump-5')]),('recording-expanded','settings-recording-expanded',None),('summaries-expanded','settings-summaries-expanded',None),('helpers-expanded','settings-helpers-expanded',None),('speech-expanded','settings-speech-expanded',None),('privacy-expanded','settings-privacy',[tap('notes-privacy-details')]),('diagnostics-expanded','settings-diagnostics',None),('sync-expanded','settings-sync-expanded',None),('people-editor','people-team-editor',[tap('team-advanced')])]:
    jobs.append((name,scene,width,theme,2200 if 'expanded' in name or 'editor' in name else 900,events))
 # Existing legacy search proof keeps its original 800px viewport and exact interaction.
 events=json.loads((ROOT/'tools/ui-scripts/settings-search.json').read_text())['events'][:-1]
 jobs.append(('search','settings-search',1280,'light',800,events))
 if args.search_keys:
  mapping=json.loads((ROOT/'tools/ui-fixtures/settings-search-map.json').read_text(encoding='utf-8'))
  jobs=[('legacy-search-'+str(i).zfill(2),'settings-search',1280,'light',900,[tap('hex-filter',.2),dict(type='type',text=item['key'],t=.4),dict(type='key',name='Enter',t=1.)]) for i,item in enumerate(mapping)]
 if args.quick:jobs=[j for j in jobs if j[0] in ('updates','hex-dictation','meetings-recording') and j[2]==1280 and j[3]=='dark']
 if args.only:jobs=[j for j in jobs if j[0] in args.only.split(',')]
 with ThreadPoolExecutor(max_workers=2) as pool:paths=list(pool.map(render,jobs))
 for name in ('updates','hex-dictation','meetings-recording'):
  source=OUT/f'settings-{name}-1280-dark.png'
  if source.exists():shutil.copyfile(source,OUT/f'{name}-1280-dark.png')
 (OUT/('search-render-manifest.json' if args.search_keys else 'render-manifest.json')).write_text(json.dumps(paths,indent=2)+'\n',encoding='utf-8')
 if args.search_keys:
  for i,item in enumerate(mapping):
   tree=json.loads((OUT/f'settings-legacy-search-{i:02}-1280-light.json').read_text(encoding='utf-8-sig'))
   elements=tree['elements']
   title=next(n for n in elements if n.get('key')=='settings-title')
   assert item['home'].startswith(title['text']),(item['key'],title.get('text'),item['home'])
   target=next((n for n in elements if n.get('key')==item['target']),None)
   assert target is not None,(item['key'],item['target'])
   assert target['rect'][1]<900 and target['rect'][1]+target['rect'][3]>0,(item['key'],target['rect'])
  print('PASS: all 49 legacy keys search to their approved page and visible destination',flush=True)
 print(f'PASS: {len(paths)} isolated renders',flush=True)
if __name__=='__main__':main()
