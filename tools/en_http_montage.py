"""One public production-route replay. No recording, owner settings or notes calls."""
import argparse, hashlib, importlib.util, json, os, shutil, subprocess, tempfile, time, wave
from pathlib import Path
import psutil

REPO = Path(__file__).resolve().parents[1]
OUT = Path('E:/Personal-Projects/reviews/hex-meetings-2026-11-16/en-http')
OLD = REPO.parent/'montage-nospeech/artifacts/montage-nospeech2'
MANIFEST = Path('E:/Personal-Projects/reviews/hex-meetings-2026-11-14/s56/s45-source/montage.json')
EXE = REPO/'target/x86_64-pc-windows-gnu/release/sidenote.exe'
FLAGS = subprocess.CREATE_NO_WINDOW
def read(p): return json.loads(Path(p).read_text(encoding='utf-8-sig'))
def save(p, value): Path(p).write_text(json.dumps(value, indent=2, ensure_ascii=False), encoding='utf-8')
def sha(p):
    with Path(p).open('rb') as f: return hashlib.file_digest(f, 'sha256').hexdigest()
def command(argv, env=None, timeout=60):
    p = subprocess.run(list(map(str, argv)), env=env, capture_output=True, timeout=timeout, creationflags=FLAGS)
    with (OUT/'commands.jsonl').open('a') as f:
        f.write(json.dumps(dict(argv=list(map(str, argv)), exit_code=p.returncode))+'\n')
    return p
def prepare():
    assert not (OUT/'fixture.json').exists(), 'fixture already frozen'
    manifest = read(MANIFEST)
    assert sha(manifest['path']) == manifest['sha256']
    old = Path(read(OLD/'fixture.json')['root'])
    root = Path(tempfile.mkdtemp(prefix='sidenote-en-http-'))
    for name in ['config-template.json', 'baseline.json']: shutil.copyfile(old/name, root/name)
    support = root/'hex-public'
    (support/'models').mkdir(parents=True)
    pin = command(['hex.com','model','status','parakeet_unified_en','--language','en'])
    assert pin.returncode == 0
    model = Path(pin.stdout.decode().strip().split('\t', 3)[3])
    for p in model.parent.iterdir():
        if p.is_file() and p.name.startswith(('parakeet-unified-en-', 'silero-vad-')):
            shutil.copyfile(p, support/'models'/p.name)
    # Only the embedded child uses the empty public support root. Control probes remain read-only.
    source = root/'bridge.c'
    source.write_text('#define REAL L'+json.dumps(shutil.which('hex.com'))+'\n'+r'''
#include <windows.h>
#include <wchar.h>
#include <stdlib.h>
int wmain(int argc,wchar_t **argv){
 if(argc>1&&!wcscmp(argv[1],L"service"))SetEnvironmentVariableW(L"HEX_APPLICATION_SUPPORT_DIR",_wgetenv(L"EN_HTTP_SUPPORT_DIR"));
 wchar_t *r=GetCommandLineW();if(*r==L'"'){r++;while(*r&&*r!=L'"')r++;if(*r)r++;}else while(*r&&*r>L' ')r++;
 size_t n=wcslen(REAL)+wcslen(r)+4;wchar_t *cmd=calloc(n,sizeof(wchar_t));swprintf(cmd,n,L"\"%ls\"%ls",REAL,r);
 STARTUPINFOW si={0};PROCESS_INFORMATION pi={0};si.cb=sizeof(si);si.dwFlags=STARTF_USESTDHANDLES;
 si.hStdInput=GetStdHandle(STD_INPUT_HANDLE);si.hStdOutput=GetStdHandle(STD_OUTPUT_HANDLE);si.hStdError=GetStdHandle(STD_ERROR_HANDLE);
 if(!CreateProcessW(REAL,cmd,0,0,TRUE,CREATE_NO_WINDOW,0,0,&si,&pi))return 126;
 WaitForSingleObject(pi.hProcess,INFINITE);DWORD code=1;GetExitCodeProcess(pi.hProcess,&code);CloseHandle(pi.hThread);CloseHandle(pi.hProcess);free(cmd);return code;}
''', encoding='utf-8')
    gcc = Path('E:/Personal-Projects/tools/hex-meetings-gnu/w64devkit/bin/gcc.exe')
    env = dict(os.environ, PATH=str(gcc.parent)+os.pathsep+os.environ['PATH'])
    assert command([gcc,'-municode','-Os','-static','-s',source,'-o',root/'hex.com'],env).returncode == 0
    save(OUT/'fixture.json', dict(root=str(root), audio=manifest['path'], sha256=manifest['sha256'],
        model_sha256=sha(model), manifest_sha256=sha(MANIFEST), support=str(support)))
def preflight():
    paper=read(OUT/'lengths-paper.json')
    save(OUT/'preflight.json',dict(speech_limit=378.67,expected=paper['http_expected_seconds'],
        source='lengths-paper.json: 20 actual production spans per language; calls*a + audio*b plus measured setup',
        joining_assumption=paper['joining_assumption'],owner_stop_to_notes_target_seconds=[180,240],
        notes='not measured',audio_seconds=1800,profile='release',attempts=1))
def run(pilot):
    f = read(OUT/'fixture.json');root = Path(f['root']);name = 'pilot' if pilot else 'b'
    assert not (OUT/(name+'-started.json')).exists(), 'one attempt only'
    assert read(OUT/'preflight.json')['expected'] == read(OUT/'lengths-paper.json')['http_expected_seconds']
    assert read(OUT/'preflight.json')['expected'] < 378.67
    case = root/name;state = case/'state';meetings = case/'meetings'
    state.mkdir(parents=True);(case/'bin').mkdir();shutil.copyfile(root/'hex.com',case/'bin/hex.com')
    config = read(root/'config-template.json');save(state/'config.json',config)
    save(state/'meeting-languages.json',['en','hi','te'])
    ident = str(int(time.time()*1000))+'-16';folder = meetings/ident;folder.mkdir(parents=True)
    if pilot:
        with wave.open(f['audio'],'rb') as source, wave.open(str(folder/'system.wav'),'wb') as dest:
            dest.setparams(source.getparams());dest.writeframes(source.readframes(30*16000))
    else:
        shutil.copyfile(f['audio'],folder/'system.wav');assert sha(folder/'system.wav') == f['sha256']
    rows = read(root/'baseline.json');rows = rows[:1] if pilot else rows
    capture = Path(read(OLD/'fixture.json')['root'])
    for i, row in enumerate(rows):
        if row['text'].strip(): continue
        reply = read(capture/f'baseline-{i:02}-response.json')
        assert reply.get('version') == 4 and reply.get('status') == 'error' and reply.get('code') == 'no-speech'
        row['baseline_reply'] = reply
    (folder/'transcript.ndjson').write_text(''.join(json.dumps(r,ensure_ascii=False)+'\n' for r in rows),encoding='utf-8')
    save(folder/'quick-title.json',dict(title='Public montage'))
    status = command(['hex.com','status','--json']);assert status.returncode == 0
    status = json.loads(status.stdout)
    assert status['meeting']['state'] == status['capture']['state'] == 'idle'
    assert status['capture']['pending_jobs'] == 0 and status['model']['preparing'] is None
    version = command(['hex.com','--version']).stdout.decode().strip()
    stopped = int(time.time()*1000);duration = 30 if pilot else 1800
    save(folder/'manifest.json',dict(status='complete',created_at_ms=stopped-duration*1000,
        ended_at_ms=stopped,duration_ms=duration*1000,system=dict(first_arrival_us=0)))
    env = dict(os.environ, SIDENOTE_DATA_DIR=str(state),SIDENOTE_STATE_DIR=str(state),
        SIDENOTE_HEX_DIR=str(meetings),EN_HTTP_SUPPORT_DIR=f['support'])
    argv = [EXE,'--route-pending','--automatic','--meeting-id',ident,'--data-dir',state,'--meetings-dir',meetings,
        '--hex-cli',case/'bin/hex.com']
    save(OUT/(name+'-started.json'),dict(argv=list(map(str,argv)),stopped_ms=stopped,
        binary_sha256=sha(EXE),config_sha256=sha(state/'config.json')))
    start=time.perf_counter();p=subprocess.Popen(list(map(str,argv)),env=env,stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,creationflags=FLAGS)
    owned={}
    with (OUT/(name+'-load.jsonl')).open('w') as log:
        while p.poll() is None:
            try:
                for child in psutil.Process(p.pid).children(recursive=True):
                    if child.name().lower() in ['hex.exe','hex.com']:
                        owned[child.pid]=dict(pid=child.pid,created=child.create_time(),name=child.name())
            except psutil.NoSuchProcess: pass
            log.write(json.dumps(dict(utc=time.time(),cpu=psutil.cpu_percent(),ram_pct=psutil.virtual_memory().percent))+'\n');log.flush()
            try: p.wait(timeout=1)
            except subprocess.TimeoutExpired: pass
            if time.perf_counter()-start > 3700:
                for child in psutil.Process(p.pid).children(recursive=True): child.kill()
                p.kill();p.wait();break
    transport=state/'local/router-work'/ident/'batches/english-transport.json'
    survivors=[]
    for pid, child in owned.items():
        try:
            if psutil.Process(pid).create_time()==child['created']: survivors.append(child)
        except psutil.NoSuchProcess: pass
    version_after=command(['hex.com','--version']).stdout.decode().strip()
    save(OUT/(name+'-run.json'),dict(id=ident,root=str(case),stopped_ms=stopped,seconds=time.perf_counter()-start,
        argv=list(map(str,argv)),hex_version_after=version_after,english_transport=read(transport) if transport.exists() else None,
        observed_owned_hex=list(owned.values()),surviving_owned_hex=survivors,
        exit_code=p.returncode,hex_version=version,binary_sha256=sha(EXE),audio_seconds=duration,profile='release',
        omitted=['physical Stop/listener dispatch','pre-Stop capture','notes','translation','title','tasks','export','external comparison','install/download/login']))
    print(name,'exit',p.returncode,flush=True)
    assert not survivors, 'owned Hex child survived the run'
def score():
    path=REPO.parent/'montage-nospeech/tools/montage_score.py'
    spec=importlib.util.spec_from_file_location('frozen_score',path);module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)
    module.ROOT=OUT;module.main()
if __name__ == '__main__':
    mode=argparse.ArgumentParser();mode.add_argument('mode',choices=['prepare','preflight','pilot','run','score'])
    mode.add_argument('--out',type=Path,default=OUT);args=mode.parse_args();OUT=args.out
    OUT.mkdir(parents=True,exist_ok=True)
    if args.mode=='prepare': prepare()
    elif args.mode=='preflight': preflight()
    elif args.mode=='score': score()
    else: run(args.mode=='pilot')
