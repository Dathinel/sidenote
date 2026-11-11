"""Run the requested Settings build gates from this worktree."""
from pathlib import Path
import json,os,subprocess,time,sys
ROOT=Path(__file__).resolve().parents[1]
OUT=ROOT/'artifacts/settings-build/gates'
OUT.mkdir(parents=True,exist_ok=True)
ENV=os.environ|{'PATH':r'E:\Personal-Projects\tools\hex-meetings-gnu\w64devkit\bin;'+os.environ['PATH'],'CARGO_TARGET_DIR':str(ROOT/'target')}
COMMANDS=[('fmt',['cargo','fmt','--all','--check']),('clippy',['cargo','clippy','--workspace','--locked','--all-targets','--','-D','warnings']),('clippy-asr',['cargo','clippy','--workspace','--locked','--all-targets','--features','asr-runtime','--','-D','warnings']),('test',['cargo','test','--workspace','--locked']),('test-asr',['cargo','test','--workspace','--locked','--features','asr-runtime']),('build',['cargo','build','--locked']),('mcp-smoke',[sys.executable,'tools/mcp-smoke.py',str(ROOT/'target/x86_64-pc-windows-gnu/debug/sidenote.exe')])]
order=['fmt','clippy','clippy-asr','test','test-asr','build','mcp-smoke']
COMMANDS.sort(key=lambda item:order.index(item[0]))
results=[]
for name,cmd in COMMANDS:
 print('Running '+name,flush=True);start=time.monotonic()
 with (OUT/(name+'.log')).open('w',encoding='utf-8') as log:
  result=subprocess.run(cmd,cwd=ROOT,env=ENV,stdout=log,stderr=log,creationflags=subprocess.CREATE_NO_WINDOW)
 row=dict(gate=name,command=cmd,exit_code=result.returncode,elapsed_seconds=round(time.monotonic()-start,2));results.append(row)
 (OUT/'results.json').write_text(json.dumps(results,indent=2)+'\n',encoding='utf-8')
 print(('PASS ' if result.returncode==0 else 'FAIL ')+name+' '+str(row['elapsed_seconds'])+'s',flush=True)
 if result.returncode:
  print((OUT/(name+'.log')).read_text(encoding='utf-8')[-7000:],flush=True)
  sys.exit(result.returncode)
