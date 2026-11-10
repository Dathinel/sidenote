"""Render the Memory review matrix through the native offscreen app."""
import argparse
import hashlib
import json
import tempfile
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path
import render_people_review as native
from prepare_memory_fixture import prepare

ROOT=native.ROOT
OUT=ROOT/'artifacts/pages-memory-round1'
native.OUT=OUT
tap=native.tap
key=native.key


def render(exe,name,case='one',width=1280,theme='light',events=None,state=None):
    (OUT/'state').mkdir(parents=True,exist_ok=True)
    state=state or Path(tempfile.mkdtemp(prefix=name+'-',dir=OUT/'state'))
    meetings=prepare(state,count=5 if case=='five' else 1,empty=case=='empty')
    if case=='browse':events=[tap('memory-browse')]
    if case=='browse-filters':events=[tap('memory-browse'),tap('memory-filters',.7)]
    if case=='undo':
        raw=json.loads((state/'memory/records.json').read_text())
        ident=raw['proposals'][0]['id']
        events=[tap('memory-accept-'+ident),tap('memory-undo-0',2)]
    return native.render(exe,name,'memory',width,theme,events,state,meetings)


def main():
    p=argparse.ArgumentParser();p.add_argument('exe',type=Path);p.add_argument('--quick',action='store_true');a=p.parse_args()
    cases=[('memory','one'),('memory-5','five'),('memory-empty','empty'),('memory-browse','browse'),('memory-browse-filters','browse-filters'),('memory-undo','undo'),('settings-general','settings')]
    jobs=[(n,c,w,t) for n,c in cases for w in ([1280] if a.quick else [1280,640]) for t in (['light'] if a.quick else ['light','dark'])]
    def run(job):
        n,c,w,t=job
        if c=='settings':return native.render(a.exe.resolve(),n,'settings-general',w,t)
        return render(a.exe.resolve(),n,c,w,t)
    with ThreadPoolExecutor(max_workers=2) as pool:rows=list(pool.map(run,jobs))
    (OUT/'render-index.json').write_text(json.dumps(dict(binary_sha256=hashlib.sha256(a.exe.read_bytes()).hexdigest(),rows=rows),indent=2))
    print(f'PASS: {len(rows)} native Memory renders',flush=True)


if __name__=='__main__':main()
