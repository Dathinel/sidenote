"""Alternating public comparison at actual production span lengths; no recording controls."""
import argparse, concurrent.futures, json, os, queue, shutil, subprocess, sys, threading, time, urllib.request
from pathlib import Path
import psutil
from en_http_costs import OUT, read, save
REPO=Path(__file__).resolve().parents[1]
DRIVER=REPO/'target/x86_64-pc-windows-gnu/release/examples/en_http_native_cost.exe'
HEX=shutil.which('hex.com');FLAGS=subprocess.CREATE_NO_WINDOW
class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self,*args,**kwargs):return None
def run(args,env=None,timeout=30):
    return subprocess.run(list(map(str,args)),env=env,capture_output=True,timeout=timeout,creationflags=FLAGS)
def line_reader(pipe,q):
    for line in iter(pipe.readline,b''):q.put(line)
    q.put(b'')
class Service:
    def __init__(self,host,env,version):
        self.version=version
        self.p=subprocess.Popen([str(host)],env=env,stdin=subprocess.PIPE,stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,creationflags=FLAGS)
        self.q=queue.Queue();threading.Thread(target=line_reader,args=(self.p.stdout,self.q),daemon=True).start()
        try:
            self.endpoint=json.loads(self.q.get(timeout=30));e=self.endpoint
            assert e['apiVersion']=='2' and e['url'].startswith('http://127.0.0.1:')
            child=psutil.Process(e['pid']);self.identity=(child.pid,child.create_time())
            assert child.pid in [p.pid for p in psutil.Process(self.p.pid).children(recursive=True)]
            self.opener=urllib.request.build_opener(urllib.request.ProxyHandler({}),NoRedirect())
            request=urllib.request.Request(e['url']+'/health',headers={'Authorization':'Bearer '+e['token']})
            with self.opener.open(request,timeout=10) as reply:self.health=json.loads(reply.read())
            assert self.health['apiVersion']=='2';self.version=self.health['version']
        except BaseException:self.close();raise
    def call(self,wav,capture):
        started=time.perf_counter()
        e=self.endpoint
        request=urllib.request.Request(e['url']+'/transcriptions?model=parakeet_unified_en&language=en',
            data=Path(wav).read_bytes(),headers={'Authorization':'Bearer '+e['token'],'Content-Type':'audio/wav'})
        # No discovery credential is opened. Proxy and redirect handlers are disabled.
        with self.opener.open(request,timeout=180) as response:
            value=json.loads(response.read(8*1024*1024+1))
        seconds=time.perf_counter()-started
        assert isinstance(value['transcript'],str)
        if value['transcript'].strip() and not capture.exists():
            save(capture,value);save(capture.with_suffix('.version.json'),dict(hex_version=self.version))
        return seconds,not value['transcript'].strip()
    def close(self):
        if self.p.stdin and not self.p.stdin.closed:self.p.stdin.close()
        try:self.p.wait(timeout=10)
        except subprocess.TimeoutExpired:self.p.kill();self.p.wait(timeout=5)
        if hasattr(self,'identity'):
            try:
                p=psutil.Process(self.identity[0])
                if p.create_time()==self.identity[1]:
                    try:p.wait(timeout=5)
                    except psutil.TimeoutExpired:
                        p.kill();p.wait(timeout=5);raise AssertionError('owned service needed extra endpoint cleanup')
            except psutil.NoSuchProcess:pass
        if hasattr(self,'endpoint'):self.endpoint['token']=''
class Driver:
    def __init__(self,root,lang,env,capture):
        self.log=(OUT/f'lengths-{lang}-{time.time_ns()}.stderr').open('wb')
        self.p=subprocess.Popen([str(DRIVER),'--ranges',str(root/(lang+'.json')),'--config',str(root/'config.json'),
            '--hex-cli',HEX,'--capture',str(capture),'--data-dir',str(root/'state')],env=env,
            stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=self.log,creationflags=FLAGS)
        self.q=queue.Queue();threading.Thread(target=line_reader,args=(self.p.stdout,self.q),daemon=True).start()
        try:
            loaded=self.next();assert loaded['event']=='loaded';self.load=loaded['seconds']
        except BaseException:self.abort();raise
    def next(self):
        value=self.q.get(timeout=210)
        assert value,'production driver ended before reply'
        return json.loads(value)
    def call(self):
        self.p.stdin.write(b'range\n');self.p.stdin.flush();value=self.next();assert value['event']=='piece';return value
    def close(self):
        if not self.p.stdin.closed:self.p.stdin.close()
        value=self.next();assert value['event']=='finished'
        assert self.p.wait(timeout=10)==0;self.log.close();return value['seconds']
    def abort(self):
        if self.p.poll() is None:self.p.kill()
        self.p.wait(timeout=10);self.log.close()
def status():
    started=time.perf_counter();p=run([HEX,'status','--json'],timeout=10);elapsed=time.perf_counter()-started
    assert p.returncode==0,'Hex status unavailable; no audio dispatched'
    d=json.loads(p.stdout);safe=dict(version=d['version'],capture={k:d['capture'][k] for k in ['state','pending_jobs']},
        meeting={'state':d['meeting']['state']},model={'preparing':d['model']['preparing']})
    assert safe['capture']==dict(state='idle',pending_jobs=0) and safe['meeting']['state']=='idle' and safe['model']['preparing'] is None,'Hex comes first; benchmark stopped'
    return elapsed,safe,dict(pid=d.get('pid'),app_version=d.get('app_version'))
def sample(stop,rows):
    psutil.cpu_percent()
    while not stop.wait(0.5):
        rows.append(dict(cpu=psutil.cpu_percent(),ram_pct=psutil.virtual_memory().percent,
            builds=[p.pid for p in psutil.process_iter(['name']) if (p.info['name'] or '').lower() in ['cargo.exe','rustc.exe','cargo-clippy.exe']]))
def build_host(root):
    source=root/'host.c';source.write_text('#define HEX_COMMAND L'+json.dumps(HEX)+'\n'+(REPO/'tools/en_http_service_host.c').read_text(),encoding='utf-8')
    gcc=Path('E:/Personal-Projects/tools/hex-meetings-gnu/w64devkit/bin/gcc.exe');env=dict(os.environ,PATH=str(gcc.parent)+os.pathsep+os.environ['PATH'])
    p=run([gcc,'-municode','-Os','-static','-s',source,'-o',root/'host.exe'],env)
    assert p.returncode==0,'contained SDK host compile failed';return root/'host.exe'
def main(pilot):
    fixture=read(OUT/'lengths-fixture.json');root=Path(fixture['root']);name='lengths-pilot' if pilot else 'lengths'
    assert not (OUT/(name+'-started.json')).exists(),'preserve prior attempts'
    save(OUT/(name+'-started.json'),dict(utc=time.time(),pilot=pilot))
    env=dict(os.environ,SIDENOTE_DATA_DIR=str(root/'state'));http_env=dict(env,HEX_APPLICATION_SUPPORT_DIR=fixture['support'])
    version=run([HEX,'--version']).stdout.decode().strip();_,fields,app=status();save(OUT/'real-status-fields.json',fields)
    h=time.perf_counter();help_reply=run([HEX,'model','transcribe','--help']);help_seconds=time.perf_counter()-h
    assert help_reply.returncode==0;save(OUT/'real-help.json',dict(version=version,reply=help_reply.stdout.decode()))
    host=build_host(root);samples=[];stop=threading.Event();thread=threading.Thread(target=sample,args=(stop,samples));thread.start()
    rows=[];native={};status_times=[];service=None;top=None;result={}
    def checkpoint():
        save(OUT/(name+'-results.json'),dict(rows=rows,native=native,hex_version=version,app=app,status_seconds=status_times,
            service_start_seconds=result.get('start'),service_close_seconds=result.get('close'),help_seconds=help_seconds))
    try:
        t=time.perf_counter();service=Service(host,http_env,version);result['start']=time.perf_counter()-t
        e=service.endpoint;save(OUT/(name+'-endpoint-fields.json'),dict(version=version,reply={k:('[redacted ephemeral credential]' if k=='token' else v) for k,v in e.items()}))
        save(OUT/(name+'-health.json'),service.health)
        if not (OUT/'real-top-reply.json').exists():save(OUT/'real-top-reply.version.json',dict(hex_version=version))
        top=Driver(root,'en',env,OUT/'real-top-reply.json');english=read(root/'en.json');limit=1 if pilot else 20
        for i,r in enumerate(english[:limit]):
            seconds,_,_=status();status_times.append(seconds)
            for path in (['http_owned','top_app'] if i%2==0 else ['top_app','http_owned']):
                if path=='http_owned':elapsed,empty=service.call(r['wav'],OUT/'real-http-reply.json')
                else:v=top.call();elapsed,empty=v['seconds'],v['empty']
                rows.append(dict(path=path,index=i,seconds=elapsed,empty=empty,ok=True,audio_seconds=(r['range']['context'][1]-r['range']['context'][0])/16000));checkpoint()
        top.close();top=None
        if not pilot:
            # Public app-API probe via the documented top-level client; no microphone/dictation starts.
            top=Driver(root,'en',env,OUT/'real-top-overlap-reply.json')
            with concurrent.futures.ThreadPoolExecutor(max_workers=2) as pool:
                for i,r in enumerate(english):
                    status();a=pool.submit(service.call,r['wav'],OUT/'real-http-overlap-reply.json');b=pool.submit(top.call)
                    elapsed,empty=a.result();v=b.result();audio=(r['range']['context'][1]-r['range']['context'][0])/16000
                    rows.extend([dict(path='http_overlap',index=i,seconds=elapsed,empty=empty,ok=True,audio_seconds=audio),
                        dict(path='top_app_overlap',index=i,seconds=v['seconds'],empty=v['empty'],ok=True,audio_seconds=audio)]);checkpoint()
            top.close();top=None
        t=time.perf_counter();service.close();result['close']=time.perf_counter()-t;service=None
        for lang in ['hi','te']:
            driver=Driver(root,lang,env,OUT/('real-'+lang+'-reply.json'));native[lang]=dict(load_seconds=driver.load)
            try:
                for i,r in enumerate(read(root/(lang+'.json'))[:limit]):
                    status();v=driver.call();rows.append(dict(path='cpu_'+lang,index=i,seconds=v['seconds'],empty=v['empty'],ok=True,audio_seconds=v['audio_seconds']));checkpoint()
                native[lang]['finish_seconds']=driver.close();driver=None
            finally:
                if driver:
                    driver.abort()
        checkpoint()
    finally:
        if top:
            top.abort()
        if service:service.close()
        stop.set();thread.join();checkpoint()
        data=read(OUT/(name+'-results.json'));data['load_summary']=dict(samples=len(samples),
            cpu_min=min((r['cpu'] for r in samples),default=None),cpu_max=max((r['cpu'] for r in samples),default=None),
            ram_min=min((r['ram_pct'] for r in samples),default=None),ram_max=max((r['ram_pct'] for r in samples),default=None),
            build_pids=sorted({pid for r in samples for pid in r['builds']}))
        data['hex_version_after']=run([HEX,'--version']).stdout.decode().strip();save(OUT/(name+'-results.json'),data);save(OUT/(name+'-load.json'),samples)
    print(name,'completed',len(rows),'public calls; accuracy not measured',flush=True)
if __name__=='__main__':
    sys.stdout.reconfigure(encoding='utf-8');p=argparse.ArgumentParser();p.add_argument('--pilot',action='store_true');main(p.parse_args().pilot)
