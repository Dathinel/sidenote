"""Render People and Teams offscreen with isolated, invented data."""
from pathlib import Path
from concurrent.futures import ThreadPoolExecutor
import argparse, hashlib, json, os, shutil, subprocess, tempfile
from prepare_people_fixture import prepare

ROOT=Path(__file__).resolve().parents[1]
OUT=ROOT/'artifacts/pages-people'
ENV=os.environ.copy()
ENV['PATH']=r'E:\Personal-Projects\tools\hex-meetings-gnu\w64devkit\bin;'+ENV['PATH']

def tap(key,t=.3): return dict(type='tap',key=key,t=t)
def key(name,t): return dict(type='key',name=name,t=t)

def render(exe,name,scene,width=1280,theme='light',events=None,state=None,meetings=None):
    (OUT/'state').mkdir(parents=True,exist_ok=True)
    state=state or Path(tempfile.mkdtemp(prefix=name+'-',dir=OUT/'state'))
    state.mkdir(parents=True,exist_ok=True)
    for directory in ['appdata','localappdata','peers']:(state/directory).mkdir(exist_ok=True)
    env=ENV|{'APPDATA':str(state/'appdata'),'LOCALAPPDATA':str(state/'localappdata'),'SIDENOTE_STATE_DIR':str(state)}
    (state/'config.json').write_text(json.dumps(dict(version=1,features={k:True for k in ['tasks','memory','daily_check','projects','other_devices']})))
    (state/'projects.json').write_text(json.dumps(dict(projects=['Northwind Atlas','Northwind Field Guide'],aliases={})))
    shutil.copy2(ROOT/'tools/ui-fixtures/hex.json',state/'hex-fixture.json')
    meetings=meetings or (prepare(state) if scene.startswith('people-') else ROOT/'tools/ui-fixtures/meetings')
    base=OUT/f'{name}-{width}-{theme}'
    args=[str(exe),'--meetings-dir',str(meetings),'--data-dir',str(state),'--peers-dir',str(state/'peers'),'--scene',scene,'--theme',theme,'--size',f'{width}x800','--scale','1','--snap',str(base)+'.png','--tree',str(base)+'.json']
    if events:
        (OUT/'scripts').mkdir(exist_ok=True)
        script=OUT/'scripts'/(base.name+'.json')
        script.write_text(json.dumps(dict(events=events+[dict(t=max(e['t'] for e in events)+1,type='mark',name='end')])))
        args+=['--script',str(script),'--at','mark:end']
    with Path(str(base)+'.log').open('w') as f:
        subprocess.run(args,cwd=ROOT,env=env,stdout=f,stderr=f,creationflags=subprocess.CREATE_NO_WINDOW,timeout=120,check=True)
    print('Rendered '+base.name,flush=True)
    return dict(path=str(base)+'.png',tree=str(base)+'.json',state=str(state),width=width,theme=theme,scene=scene)

def main():
    p=argparse.ArgumentParser();p.add_argument('exe',type=Path);p.add_argument('--quick',action='store_true');a=p.parse_args()
    cases=[('people-our-team','people-our-team'),('people-external','people-external'),('people-import','people-import'),('team-editor','people-team-editor'),('people-search','people-search'),('settings-general','settings-general')]
    shots=[(name,scene,w,t) for name,scene in cases for w in ([1280] if a.quick else [1280,640]) for t in (['light'] if a.quick else ['light','dark'])]
    with ThreadPoolExecutor(max_workers=2) as pool:rows=list(pool.map(lambda shot:render(a.exe.resolve(),*shot),shots))
    (OUT/'render-index.json').write_text(json.dumps(dict(binary_sha256=hashlib.sha256(a.exe.read_bytes()).hexdigest(),rows=rows),indent=2))
    # Stable short name requested for the narrow Our team view.
    if not a.quick:shutil.copy2(OUT/'people-our-team-640-light.png',OUT/'people-640-light.png')
    print(f'PASS: {len(rows)} People renders',flush=True)
if __name__=='__main__':main()
