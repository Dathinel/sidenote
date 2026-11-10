"""Invented memory records and real pending proposals for the native review harness."""
import json
from datetime import datetime, timedelta
from pathlib import Path
from prepare_check_fixture import sources_fp, proposal_id


def prepare(root, count=1, empty=False):
    root=Path(root)
    meetings=root/'memory-meetings'
    now=datetime.now().astimezone()
    for i,title in enumerate(['Workshop planning','Atlas review']):
        folder=meetings/f'180000000000{i}-1'
        folder.mkdir(parents=True,exist_ok=True)
        (folder/'english-summary.json').write_text(json.dumps(dict(schema=2,title=title,people=['Alex Chen','Morgan Lee'],context=['Discuss an invented workshop demo.'],todo=[])))
        (folder/'manifest.json').write_text(json.dumps(dict(status='complete',created_at_ms=int((now-timedelta(days=1-i)).timestamp()*1000),duration_ms=1800000)))
    path=root/'memory/records.json'
    if path.exists():return meetings
    path.parent.mkdir(parents=True,exist_ok=True)
    stamp=(now-timedelta(hours=2)).isoformat()
    sources=[dict(type='meeting',reference='[[1800000000000-1@00:05]]')]
    texts=['The demo is on Thursday.','Use a paper prototype for the demo.','Alex sends the sample pack.','The venue reply is still open.','Review the printed samples on Thursday.']
    records=[]
    for i in range(0 if empty else 27):
        records.append(dict(id=f'memory-{i}',type=['decision','project','commitment','question','story'][i%5],text=texts[i] if i<len(texts) else f'Workshop reference {i+1}: keep the approved paper sample.',project='Northwind Field Guide',people=['Alex Chen'],sources=sources,first_seen=stamp,last_confirmed=None,status='active',basis='source_supported',supersedes=None,updated_at=stamp,source_fingerprint=sources_fp(sources),source_fingerprints=[sources_fp(sources)]))
    proposals=[]
    for i in range(0 if empty else count):
        target=records[i]
        suggestion='The demo is on Friday.' if i==0 else f'Workshop decision {i+1}: use the revised sample.'
        pr=dict(id='',op='supersede',target_id=target['id'],text=suggestion,record_type=target['type'],project=target['project'],people=target['people'],sources=[dict(type='meeting',reference='[[1800000000001-1@00:05]]')],rationale='The latest meeting changed this plan. Both source meetings are shown below.',target_updated_at=stamp)
        pr['id']=proposal_id(pr)
        proposals.append(dict(pr,status='pending',created_at=stamp,decided_at=None))
    events=[] if empty else [dict(record_id='memory-1',operation='confirm',at=stamp,proposal_id=None)]
    path.write_text(json.dumps(dict(schema_version=1,records=records,proposals=proposals,events=events,migration=dict(version=1,source_path='synthetic',fingerprint='synthetic',accepted_at=stamp)),indent=2))
    return meetings
