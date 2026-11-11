"""Render the local settle history using real write-ahead mutations and invented meetings."""
import argparse,hashlib,json,tempfile
from concurrent.futures import ThreadPoolExecutor
from datetime import datetime
from pathlib import Path
import render_people_review as native
ROOT=native.ROOT
OUT=ROOT/'artifacts/memory-settle'
native.OUT=OUT
tap=native.tap
key=native.key
ID='1800000000000-1'

def prepare(state,many=False):
    folder=state/'settle-meetings'/ID
    if folder.exists():return folder.parent
    folder.mkdir(parents=True)
    stamp=datetime.now().astimezone().isoformat()
    tasks=[dict(id=f'task-{i}',task=t,title=t,owner=None,due=None,due_default=False,status='todo',at=[['00:05','01:10'][i%2]]) for i,t in enumerate(['Send the sample pack','Confirm the room booking','Share the approved copy','Book the customer interviews','Review the workshop plan'][:5 if many else 2])]
    (folder/'manifest.json').write_text(json.dumps(dict(status='complete',title='Workshop planning',created_at_ms=int(datetime.now().timestamp()*1000),duration_ms=1800000)))
    (folder/'english-summary.json').write_text(json.dumps(dict(schema=2,title='Workshop planning',people=['Alex Chen','Morgan Lee'],context=['Plan the Northwind workshop.'],todo=tasks)))
    (folder/'english-transcript.md').write_text('[00:00:05.000] Alex Chen (English): I will send the sample pack on Friday.\n[00:01:10.000] Morgan Lee (English): I will confirm the room booking.\n')
    (state/'memory').mkdir(exist_ok=True)
    (state/'memory/records.json').write_text(json.dumps(dict(schema_version=1,records=[],proposals=[],events=[],migration=dict(version=1,source_path='synthetic',fingerprint='synthetic',accepted_at=stamp))))
    return folder.parent

def render(exe,name,case='settled',theme='light',events=None,state=None,width=1280):
    (OUT/'state').mkdir(parents=True,exist_ok=True)
    state=state or Path(tempfile.mkdtemp(prefix=name+'-',dir=OUT/'state'))
    meetings=prepare(state,many=case=='all')
    if case=='undo':events=[tap('settle-undo-0',.4)]
    if case=='all':events=[tap('settle-see-all',.4)]
    return native.render(exe,name,'memory-settled',width,theme,events,state,meetings)

def main():
    p=argparse.ArgumentParser();p.add_argument('exe',type=Path);a=p.parse_args();exe=a.exe.resolve()
    cases=[('memory-settled','settled'),('memory-settle-undo','undo'),('memory-settle-all','all')]
    with ThreadPoolExecutor(max_workers=2) as pool:rows=list(pool.map(lambda job:render(exe,*job),[(n,c,t) for n,c in cases for t in ['light','dark']]))
    for w in [1280,640]:
        for theme in ['light','dark']:rows.append(native.render(exe,'settings-general','settings-general',w,theme))
    (OUT/'render-index.json').write_text(json.dumps(dict(binary_sha256=hashlib.sha256(exe.read_bytes()).hexdigest(),rows=rows),indent=2))
    print('PASS: six Memory and four Settings renders',flush=True)
if __name__=='__main__':main()
