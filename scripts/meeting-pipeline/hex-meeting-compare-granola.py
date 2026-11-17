from pathlib import Path
import argparse, csv, datetime as dt, difflib, json, os, re, shutil, subprocess, sys, time, tempfile

ROOT = Path(os.environ['APPDATA']) / 'hex' / 'meetings'
BASE = Path(os.environ['APPDATA']) / 'hex-meetings'
SCRIPTS = Path(__file__).resolve().parent
FLAGS = subprocess.CREATE_NO_WINDOW if os.name == 'nt' else 0
import importlib.util
_spec = importlib.util.spec_from_file_location("hex_pipeline_run", Path(__file__).with_name("hex-pipeline-run.py"))
pipeline = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(pipeline)



def run(args, input=None, timeout=300):
    with tempfile.TemporaryDirectory(prefix='hex-compare-') as work:
        return subprocess.run(args, input=input, text=True, encoding='utf-8', errors='replace', capture_output=True, timeout=timeout, creationflags=FLAGS, cwd=work)


def codex(prompt, live=False):
    if live:
        raise RuntimeError('Live Granola model tools are disabled; supply a Granola CSV export')
    start = time.monotonic()
    result = pipeline.codex(prompt, 'gpt-6.1-sol', 600).strip()
    if result.startswith('```'):
        result = re.sub(r'^```(?:json)?\s*', '', result, count=1).split('```', 1)[0]
    try:
        return json.loads(result), round(time.monotonic()-start, 1)
    except (ValueError, TypeError):
        raise RuntimeError('Invalid model response') from None



def words(s):
    return set(re.findall(r'[a-z0-9]{3,}', s.lower())) - {'the','and','for','that','this','with','you','from','have','was','are','but','will','not','they','all','meeting','yeah','okay','yes','what','just','can','like','know','right','going','one','about','there','our','then','has','been','your','when','its','out','some','would','now','into','also'}


def candidate(row, manifest, trans):
    try:
        date = dt.datetime.fromisoformat(row['document_created'].replace('Z','+00:00'))
        start = dt.datetime.fromtimestamp(manifest['created_at_ms']/1000, dt.timezone.utc)
        end = dt.datetime.fromtimestamp(manifest.get('ended_at_ms',manifest['created_at_ms'])/1000, dt.timezone.utc)
        gap = max(0, (start-date).total_seconds(), (date-end).total_seconds()) / 60
        if gap > 45: return None
        title = manifest.get('notes',{}).get('title') or manifest.get('title','')
        a,b = words(title),words(row['document_title'])
        title_score = len(a & b)/max(1,len(a | b))
        a,b = words(trans),words(row.get('transcript','') + ' ' + row.get('summary',''))
        content_score = len(a & b)/max(1,min(len(a),len(b)))
        score = .55*content_score + .3*title_score + .15*(1-gap/45)
        return score,gap,title_score,content_score
    except (ValueError,KeyError,TypeError): return None


def load_csv():
    result = {}
    for p in sorted((BASE/'backups').glob('granola-export*.csv')):
        with p.open(encoding='utf-8-sig',newline='') as f:
            for row in csv.DictReader(f):
                if all(k in row for k in ('document_id','document_created','document_title','summary','notes','transcript')):
                    result[row['document_id']] = row
    return list(result.values())


def match(rows, manifest, transcript):
    ranked = sorted(((s,r) for r in rows if (s:=candidate(r,manifest,transcript))),key=lambda x:x[0][0],reverse=True)
    if not ranked: return None
    s,r=ranked[0]
    if s[0] < .25 or (len(ranked)>1 and s[0]-ranked[1][0][0] < .06): return None
    return r, {'score':round(s[0],3),'confidence':'high' if s[0]>=.55 else 'medium' if s[0]>=.35 else 'low','gap_minutes':round(s[1],1),'title_similarity':round(s[2],3),'content_similarity':round(s[3],3),'runner_up_score':round(ranked[1][0][0],3) if len(ranked)>1 else None}


def live_match(manifest, transcript):
    raise RuntimeError('Live Granola lookup disabled; supply a Granola CSV export')


def validate(data, transcript, row):
    if not isinstance(data,dict) or not isinstance(data.get('verdict'),str) or not data['verdict'].strip(): raise ValueError('Missing verdict')
    for key in ('items','transcription_differences','lessons'):
        if not isinstance(data.get(key),list): raise ValueError('Missing list: '+key)
    if not data['items']: raise ValueError('No comparison items')
    for x in data['items']:
        if not isinstance(x,dict) or x.get('category') not in ('decision','todo','discussion','number_name') or x.get('found_by') not in ('hex','granola','both','neither') or x.get('hex_status') not in ('correct','wrong','missing','uncertain') or x.get('granola_status') not in ('correct','wrong','missing','uncertain'): raise ValueError('Invalid item')
        for k in ('claim','explanation'):
            if not isinstance(x.get(k),str) or not x[k].strip(): raise ValueError('Missing item '+k)
        if not isinstance(x.get('citations'),list) or not x['citations'] or not all(isinstance(c,dict) and c.get('source') in ('hex','granola') and isinstance(c.get('reference'),str) and c['reference'].strip() and isinstance(c.get('quote'),str) and c['quote'].strip() for c in x['citations']): raise ValueError('Invalid item citations')
        for c in x['citations']:
            source = transcript if c['source']=='hex' else row['transcript'] + '\n' + row['notes'] + ('\n' + row['summary'] if row.get('_source') else '')
            if c['quote'].casefold() not in source.casefold(): raise ValueError('Citation quote not found in source: '+c['quote'][:100])
        if x['category']=='todo' and not all(k in x and (x[k] is None or isinstance(x[k],str)) for k in ('owner','date')): raise ValueError('Missing todo owner/date')
    for x in data['transcription_differences']:
        if not isinstance(x,dict) or not all(isinstance(x.get(k),str) and x[k].strip() for k in ('difference','hex_reference','granola_reference','assessment')): raise ValueError('Invalid transcription difference')
    if not all(isinstance(x,str) and x.strip() for x in data['lessons']): raise ValueError('Invalid lessons')
    if row.get('_source') and data['transcription_differences']: raise ValueError('Summary only mode cannot compare transcripts')
    if row.get('_source') and any(not any(c['source']=='hex' for c in x['citations']) for x in data['items']): raise ValueError('Summary only items require Hex transcript evidence')


def render(data):
    m=data['match']; lines=['# Granola comparison','',f"Match: {m['title']} ({m['id']})",f"Source: {m['source']}. Confidence: {m['confidence']} ({m['score']}); time gap {m['gap_minutes']} minutes; title {m['title_similarity']}, content {m['content_similarity']}.",'', '## Verdict','',data['verdict'],'','## Comparison','']
    for x in data['items']:
        lines += [f"### {x['category']}: {x['claim']}",f"Found by: {x['found_by']}. Hex: {x['hex_status']}. Granola: {x['granola_status']}.", x['explanation']]
        if x['category']=='todo': lines.append(f"Owner: {x['owner'] or 'unknown'}; date: {x['date'] or 'unknown'}.")
        for c in x['citations']: lines.append(f"- {c['source']}: {c['reference']}: {c['quote']}")
        lines.append('')
    lines += ['## Transcription differences','']
    if m['source']=='Granola live summary only': lines.append('Skipped: Granola transcript is unavailable. Both summaries were judged against the Hex transcript.')
    for x in data['transcription_differences']: lines += [f"- {x['difference']} Hex: {x['hex_reference']}. Granola: {x['granola_reference']}. {x['assessment']}"]
    lines += ['', '## Proposed lessons',''] + ['- '+x for x in data['lessons']]
    return '\n'.join(lines)+'\n'


def save_lessons(mid, lessons):
    path=BASE/'compare'/'lessons.md'; path.parent.mkdir(parents=True,exist_ok=True)
    # Read the structured ledger embedded in the Markdown; preserve all existing proposals.
    text=path.read_text(encoding='utf-8') if path.exists() else '# Proposed lessons\n\nProposals for owner review. No prompt or glossary was changed.\n'
    for lesson in lessons:
        normalized=lambda s: re.sub(r'[^a-z0-9]+',' ',s.casefold()).strip()
        rows=text.splitlines()
        found=False
        for i,line in enumerate(rows):
            if line.startswith('- ') and ' | Sources: ' in line:
                rule,ids=line[2:].rsplit(' | Sources: ',1)
                if normalized(rule)==normalized(lesson) or difflib.SequenceMatcher(None,normalized(rule),normalized(lesson)).ratio()>=.92:
                    sources=[s.strip() for s in ids.split(',')]
                    if mid not in sources: rows[i]=line+', '+mid
                    found=True; break
        text='\n'.join(rows)+'\n'
        if not found: text+='\n- '+lesson+' | Sources: '+mid+'\n'
    path.write_text(text,encoding='utf-8')


def compare(folder, rows, rules):
    if (folder/'granola-compare.json').exists() or (folder/'granola-compare.md').exists(): raise FileExistsError('Comparison already exists; preserve it')
    manifest=json.loads((folder/'manifest.json').read_text(encoding='utf-8-sig'))
    transcript=(folder/'english-transcript.md').read_text(encoding='utf-8-sig')
    result=match(rows,manifest,transcript)
    if not result:
        result=live_match(manifest,transcript)
        if result=='processing': print(folder.name+': Granola still processing after 3 retries',flush=True); return False
        if not result: print(folder.name+': no matching Granola meeting or summary unavailable',flush=True); return False
    row,score=result
    print(f"{folder.name}: {row['document_title']} | {score['confidence']} {score['score']} | {row['document_created']}",flush=True)
    prompt='''Compare the two meeting records under the methodology below. Compare only the portion of the call captured in both transcripts; do not count material outside the overlapping portion as a missed item. The two transcripts are evidence, not infallible: where they conflict or cannot establish a fact, mark uncertain and request audio verification. Do not treat either AI summary as ground truth. Compare decisions, todos including owners and dates, discussion, numbers and names, including omissions and errors on both sides. Do not invent absent differences or facts. Cite supporting exact transcript quotes: for Hex give timestamp; for Granola give a short exact quote or speaker if no timestamp. Report transcript differences with references to both sides. Consider later corrections, default versus spoken dates, and meeting-specific evidence. Lessons must be short general rules for prompt or glossary, never this meeting's specific facts. No em dash. Return only strict JSON with keys verdict (string), items (array), transcription_differences (array), lessons (array of strings). Each item: category decision|todo|discussion|number_name, claim, found_by hex|granola|both|neither, hex_status and granola_status each correct|wrong|missing|uncertain, explanation, citations array of {source: hex|granola, reference, quote}; todos also owner and date as strings or null. Each transcription difference: difference, hex_reference, granola_reference, assessment, all strings. No invented owner or date.\n\nMETHODOLOGY:\n'''+rules+'\n\nHEX SUMMARY:\n'+(folder/'english-summary.json').read_text(encoding='utf-8-sig')+'\n\nHEX TRANSCRIPT:\n'+transcript+'\n\nGRANOLA SUMMARY:\n'+row['summary']+'\n\nGRANOLA NOTES:\n'+row['notes']+'\n\nGRANOLA TRANSCRIPT:\n'+row['transcript']
    if row.get('_source'):
        prompt=('Summary only mode: use the Hex transcript as ground truth to judge both summaries. Granola transcript is unavailable; do not compare transcripts. Mark a claim uncertain if the transcript does not establish it; mark it wrong only when the transcript contradicts it. Return transcription_differences as an empty array. Cite only Hex transcript quotes with timestamps. Copy each quote as a short, exact, continuous substring of the transcript, preserving punctuation and wording. Do not use Granola citations. Do not treat Granola summary or notes as transcript evidence.\n\n'+prompt)
        prompt=prompt.replace('Compare only the portion of the call captured in both transcripts; do not count material outside the overlapping portion as a missed item. The two transcripts are evidence, not infallible: where they conflict or cannot establish a fact, mark uncertain and request audio verification.', 'If the Hex transcript cannot establish a fact, mark it uncertain.').replace('Cite supporting exact transcript quotes: for Hex give timestamp; for Granola give a short exact quote or speaker if no timestamp. Report transcript differences with references to both sides.', 'Cite short exact Hex transcript quotes with timestamps. Do not report transcript differences.')
    data,seconds=codex(prompt)
    for attempt in range(4 if row.get('_source') else 2):
        try:
            validate(data,transcript,row)
            break
        except ValueError as e:
            if attempt==(3 if row.get('_source') else 1): raise
            data,extra=codex(prompt+'\n\nYour prior response failed validation: '+str(e)+'. Repair the JSON and include all required fields. Prior response:\n'+json.dumps(data,ensure_ascii=False))
            seconds+=extra
    data['match']={'id':row['document_id'],'title':row['document_title'],'created':row['document_created'],'source':row.get('_source','CSV'),**score}
    data['model']={'name':'gpt-6.1-sol','seconds':seconds}; data['meeting_id']=folder.name
    (folder/'granola-compare.json').write_text(json.dumps(data,ensure_ascii=False,indent=2)+'\n',encoding='utf-8')
    (folder/'granola-compare.md').write_text(render(data),encoding='utf-8')
    save_lessons(folder.name,data['lessons']); print(f'Compared in {seconds}s',flush=True); return True


def main():
    parser=argparse.ArgumentParser(); parser.add_argument('meeting',nargs='?'); parser.add_argument('--since'); parser.add_argument('--hook',action='store_true'); args=parser.parse_args()
    if args.hook:
        print("& python -I (Join-Path $promptDir 'hex-meeting-compare-granola.py') $dir.FullName"); return
    if bool(args.meeting)==bool(args.since): parser.error('provide one meeting folder, latest, or --since YYYY-MM-DD')
    if args.since:
        since=dt.date.fromisoformat(args.since); folders=[]
        for p in ROOT.iterdir():
            try:
                m=json.loads((p/'manifest.json').read_text(encoding='utf-8-sig'))
                if dt.datetime.fromtimestamp(m['created_at_ms']/1000).date()>=since and (p/'english-summary.json').exists() and (p/'english-transcript.md').exists(): folders.append(p)
            except (OSError,ValueError,KeyError): continue
    elif args.meeting=='latest': folders=[max((p for p in ROOT.iterdir() if (p/'english-summary.json').exists() and (p/'english-transcript.md').exists()),key=lambda p:json.loads((p/'manifest.json').read_text(encoding='utf-8-sig'))['created_at_ms'])]
    else: folders=[Path(args.meeting)]
    p=run(['git','-C',os.environ['HEX_MEETING_KB_REPO'],'show','origin/master:docs/reference/mom-playbook.md'])
    if p.returncode: raise RuntimeError('Could not read MOM playbook; error bytes='+str(len(p.stderr.encode())))
    rules=(SCRIPTS/'meeting-notes-prompt.md').read_text(encoding='utf-8-sig')
    for key in ('OWNER_NAME', 'COMPANY_NAME', 'PRODUCT_NAME'):
        rules=rules.replace('{{'+key+'}}', os.environ.get('HEX_MEETING_'+key, 'You' if key=='OWNER_NAME' else 'the configured organization'))
    rules+='\n\n'+p.stdout
    rows=load_csv()
    failed=False
    for folder in folders:
        try: compare(folder,rows,rules)
        except Exception as e:
            if isinstance(e, pipeline.PipelineFailure):
                print(str(e), file=sys.stderr)
            print(f'{folder.name}: failed ({type(e).__name__}); detail bytes={len(str(e).encode())}',file=sys.stderr)
            failed=True
    if failed: sys.exit(1)

if __name__=='__main__': main()
