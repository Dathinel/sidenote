"""Production-length selection and nonnegative t = a + b * audio_seconds fits."""
import hashlib, json, math, shutil, statistics, tempfile, wave
from pathlib import Path
OUT=Path('E:/Personal-Projects/reviews/hex-meetings-2026-11-16/en-http')
OLD=Path('E:/Personal-Projects/hex-meetings/.worktrees/montage-nospeech/artifacts/montage-nospeech2/b-second')
def read(p): return json.loads(Path(p).read_text(encoding='utf-8-sig'))
def save(p,v): Path(p).write_text(json.dumps(v,indent=2,ensure_ascii=False),encoding='utf-8')
def prepare():
    assert not (OUT/'lengths-fixture.json').exists()
    run=read(OLD/'b-run.json');work=Path(run['root'])/'state/local/router-work'/run['id']
    plan=read(work/'batches/batch-plan.json');prior=read(OUT/'fixture.json')
    root=Path(tempfile.mkdtemp(prefix='sidenote-en-http-lengths-'));(root/'state').mkdir();selected={};counts={}
    for lang,items in plan['batches'].items():
        ranges=sorted(items.values(),key=lambda r:r['range']['context'][1]-r['range']['context'][0])
        selected[lang]=[]
        for i in range(20):
            r=dict(ranges[math.floor((i+0.5)*len(ranges)/20)])
            src=Path(r['wav']);dst=root/f'{lang}-{i:02}.wav';shutil.copyfile(src,dst)
            with wave.open(str(dst),'rb') as w:
                assert (w.getframerate(),w.getnchannels(),w.getsampwidth())==(16000,1,2)
                assert w.getnframes()==r['range']['context'][1]-r['range']['context'][0]
            r['wav']=str(dst);selected[lang].append(r)
        save(root/(lang+'.json'),selected[lang])
        seconds=lambda r:(r['range']['context'][1]-r['range']['context'][0])/16000
        counts[lang]=dict(calls=len(ranges),audio_seconds=sum(map(seconds,ranges)),
            selected_calls=20,selected_mean_audio_seconds=statistics.mean(map(seconds,selected[lang])),
            selected_min_audio_seconds=min(map(seconds,selected[lang])),selected_max_audio_seconds=max(map(seconds,selected[lang])))
    shutil.copyfile(Path(prior['root'])/'config-template.json',root/'config.json')
    save(OUT/'lengths-fixture.json',dict(root=str(root),support=prior['support'],source_plan=str(work/'batches/batch-plan.json'),
        counts=counts,selection='20 evenly spaced duration quantiles of actual production spans; no trimming or joining',
        joining_assumption='speaker_turn=None; one production range per detected span',
        old_paper_check_superseded=True))
def fit(rows):
    warm=rows[1:];x=[r['audio_seconds'] for r in warm];y=[r['seconds'] for r in warm]
    mx,my=statistics.mean(x),statistics.mean(y);sxx=sum((v-mx)**2 for v in x)
    b=sum((a-mx)*(c-my) for a,c in zip(x,y))/sxx if sxx else 0.0;a=my-b*mx
    candidates=[(my,0.0),(0.0,sum(p*q for p,q in zip(x,y))/sum(p*p for p in x))]
    if a>=0 and b>=0:candidates.append((a,b))
    a,b=min(candidates,key=lambda ab:sum((q-ab[0]-ab[1]*p)**2 for p,q in zip(x,y)))
    sse=sum((q-a-b*p)**2 for p,q in zip(x,y));sst=sum((q-my)**2 for q in y)
    cold=max(0.0,rows[0]['seconds']-a-b*rows[0]['audio_seconds'])
    return dict(seconds_per_call=a,seconds_per_audio_second=b,cold_first_excess_seconds=cold,
        fit_calls=len(warm),r_squared=1-sse/sst if sst else None,
        mean_observed_seconds_per_call=statistics.mean(r['seconds'] for r in rows),
        observed_total_seconds_per_audio_second=sum(r['seconds'] for r in rows)/sum(r['audio_seconds'] for r in rows),
        min_audio_seconds=min(r['audio_seconds'] for r in rows),max_audio_seconds=max(r['audio_seconds'] for r in rows))
def paper():
    known=fit([dict(audio_seconds=x,seconds=1+2*x) for x in [1,2,3,4]])
    assert abs(known['seconds_per_call']-1)<1e-8 and abs(known['seconds_per_audio_second']-2)<1e-8
    data=read(OUT/'lengths-results.json');fixture=read(OUT/'lengths-fixture.json');fits={}
    for path in ['http_owned','top_app','cpu_hi','cpu_te']:
        rows=[r for r in data['rows'] if r['path']==path]
        assert len(rows)==20 and all(r['ok'] for r in rows),path
        fits[path]=fit(rows)
    en={r['index']:r['audio_seconds'] for r in data['rows'] if r['path']=='http_owned'}
    assert en=={r['index']:r['audio_seconds'] for r in data['rows'] if r['path']=='top_app'}
    solo={r['index']:r['seconds'] for r in data['rows'] if r['path']=='top_app'}
    overlap={r['index']:r['seconds'] for r in data['rows'] if r['path']=='top_app_overlap'}
    assert set(overlap)==set(solo)==set(range(20))
    projected={}
    for path,lang in [('http_owned','en'),('top_app','en'),('cpu_hi','hi'),('cpu_te','te')]:
        f=fits[path];c=fixture['counts'][lang]
        total=c['calls']*f['seconds_per_call']+c['audio_seconds']*f['seconds_per_audio_second']+f['cold_first_excess_seconds']
        if path.startswith('cpu_'):total+=data['native'][lang]['load_seconds']+data['native'][lang]['finish_seconds']
        projected[path]=total
    guards=fixture['counts']['en']['calls']*statistics.mean(data['status_seconds'])+data['help_seconds']
    http_total=projected['http_owned']+projected['cpu_hi']+projected['cpu_te']+guards+data['service_start_seconds']+data['service_close_seconds']
    top_total=projected['top_app']+projected['cpu_hi']+projected['cpu_te']+guards
    output=dict(fits=fits,production=fixture['counts'],projected_language_seconds=projected,
        guards_seconds=guards,service_start_seconds=data['service_start_seconds'],service_close_seconds=data['service_close_seconds'],
        speech_limit_seconds=378.67,http_expected_seconds=http_total,top_expected_seconds=top_total,
        fits_speech_limit=http_total<378.67,owner_stop_to_notes_target_seconds=[180,240],
        stop_to_notes_expected='not measured',notes='not measured',
        timing_accuracy='Length-matched timing comparison only; accuracy not measured in this comparison.',
        formula='calls*a + audio_seconds*b + first-cold excess + one-time loader/start/finish costs; no double counting of mean seconds/call',
        exclusions=['VAD/LID','range cutting and spool','model pinning outside Native.load','assembly/publication','notes','physical Stop/listener dispatch'],
        joining_assumption=fixture['joining_assumption'],load=data['load_summary'],hex_version=data['hex_version'])
    output['app_api_public_probe']=dict(paired_median_overlap_ratio=statistics.median(overlap[i]/solo[i] for i in range(1,20)),
        solo_mean_seconds=statistics.mean(solo.values()),overlap_mean_seconds=statistics.mean(overlap.values()),
        source='top-level client of the running app, same public pieces, concurrent with owned helper',
        live_dictation_latency='not measured; no recording or microphone session was started')
    save(OUT/'lengths-paper.json',output);print(json.dumps(output,indent=2))
if __name__=='__main__':
    import sys
    OUT.mkdir(parents=True,exist_ok=True)
    prepare() if sys.argv[1]=='prepare' else paper()
