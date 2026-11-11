"""Render finished meetings using isolated, invented data and the native offscreen app."""
import argparse, hashlib, json, shutil, tempfile
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path
import render_people_review as native
from prepare_check_fixture import sources_fp
ROOT=native.ROOT
OUT=ROOT/'artifacts/pages-meeting-round1'
native.OUT=OUT
ID='1793955600000-1'
tap=native.tap
key=native.key

def prepare(state,case):
    meetings=state/'meeting-fixture'
    if meetings.exists():return meetings
    shutil.copytree(ROOT/'tools/ui-fixtures/screens/meetings',meetings)
    main=meetings/ID
    path=main/'english-summary.json'
    data=json.loads(path.read_text())
    data.update(schema=2,made_by=dict(instructions='default'))
    for i,t in enumerate(data['todo']):
        t.update(ref=f't{i}',title=t['task'],description=['Share the approved survey numbers.','Send the final pricing copy.','Make step three one screen.','Arrange five customer sessions.'][i])
    if case=='no-tasks':
        data.update(todo=[],to_settle=[])
        (main/'tasks.json').write_text(json.dumps(dict(meeting_project=None,tasks=[])))
    else:
        (main/'tasks.json').write_text(json.dumps(dict(meeting_project='Northwind Atlas',tasks=[dict(i=i,project='Northwind Atlas',mine=True) for i in range(4)])))
    if case=='long-title':
        title='Northwind onboarding review: launch readiness, pricing copy and the shorter customer sign-up flow for December'
        data['title']=title
        manifest=json.loads((main/'manifest.json').read_text());manifest['title']=title
        (main/'manifest.json').write_text(json.dumps(manifest))
    path.write_text(json.dumps(data))
    (state/'teams.json').write_text(json.dumps(dict(teams=[dict(name='Product',members=['Maya','Priya','Tom'],aliases={})])))
    mem=state/'memory';mem.mkdir(exist_ok=True)
    records=[]
    for i,meeting in enumerate([ID,'1793939400000-1']):
        sources=[dict(type='meeting',reference=f'[[{meeting}@02:10]]')]
        records.append(dict(id=f'meeting-memory-{i}',type='decision',text=['Launch in the second week of December.','Unrelated meeting decision.'][i],project='Northwind Atlas',people=['Maya'],sources=sources,first_seen='2026-11-06',last_confirmed=None,status='active',basis='source_supported',supersedes=None,updated_at='2026-11-10T10:00:00Z',source_fingerprint=sources_fp(sources),source_fingerprints=[sources_fp(sources)]))
    (mem/'records.json').write_text(json.dumps(dict(schema_version=1,records=records,proposals=[],events=[dict(record_id='meeting-memory-0',operation='confirm',at='2026-11-10T10:00:00Z',proposal_id=None)],migration=dict(version=1,source_path='synthetic',fingerprint='synthetic',accepted_at='2026-11-10T10:00:00Z'))))
    return meetings

def render(exe,name,case='finished',width=1280,theme='light',events=None,state=None):
    (OUT/'state').mkdir(parents=True,exist_ok=True)
    state=state or Path(tempfile.mkdtemp(prefix=name+'-',dir=OUT/'state'))
    meetings=prepare(state,case)
    if case=='source-hover':events=[dict(type='move',key='meeting-point-pt-0-0',t=.3)]
    if case=='transcript-sheet':events=[dict(type='move',key='meeting-point-pt-0-0',t=.3),tap('meeting-source-pt-0-0',.7),dict(type='move',x=10,y=700,t=1.0)]
    return native.render(exe,name,'meeting-reading-wide-ask' if case=='wide-ask' else 'meeting-reading-review',width,theme,events,state,meetings)

def main():
    p=argparse.ArgumentParser();p.add_argument('exe',type=Path);p.add_argument('--quick',action='store_true');a=p.parse_args()
    cases=[('meeting','finished'),('meeting-source-hover','source-hover'),('meeting-transcript-sheet','transcript-sheet'),('meeting-no-tasks','no-tasks'),('meeting-long-title','long-title'),('meeting-wide-ask','wide-ask'),('settings-general','settings')]
    jobs=[(n,c,w,t) for n,c in cases for w in ([1280] if a.quick else [1280,640]) for t in (['light'] if a.quick else ['light','dark'])]
    def run(job):
        n,c,w,t=job
        if c=='settings':return native.render(a.exe.resolve(),n,'settings-general',w,t)
        return render(a.exe.resolve(),n,c,w,t)
    with ThreadPoolExecutor(max_workers=2) as pool:rows=list(pool.map(run,jobs))
    (OUT/'render-index.json').write_text(json.dumps(dict(binary_sha256=hashlib.sha256(a.exe.read_bytes()).hexdigest(),rows=rows),indent=2))
    print(f'PASS: {len(rows)} native Meeting renders',flush=True)
if __name__=='__main__':main()
