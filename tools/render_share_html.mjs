import fs from 'node:fs/promises';
import path from 'node:path';
import {spawn,execFileSync} from 'node:child_process';
import {pathToFileURL} from 'node:url';
const [sourceArg,outArg]=process.argv.slice(2),source=path.resolve(sourceArg),out=path.resolve(outArg);
const repo=path.resolve(path.dirname(new URL(import.meta.url).pathname.replace(/^\/(\w:)/,'$1')),'..');
const edge=path.join(process.env['ProgramFiles(x86)']||'C:/Program Files (x86)','Microsoft/Edge/Application/msedge.exe');
const sleep=ms=>new Promise(resolve=>setTimeout(resolve,ms));
const profileRoot=path.join(repo,'target/edge-share-proof');await fs.mkdir(profileRoot,{recursive:true});await fs.mkdir(out,{recursive:true});
const rows=[];
for(const [width,height] of [[1280,1600],[390,2400]])for(const theme of ['light','dark']){
 const profile=await fs.mkdtemp(path.join(profileRoot,`${width}-${theme}-`));
 const proc=spawn(edge,['--headless=new','--disable-gpu','--disable-background-networking','--disable-extensions','--no-first-run','--no-default-browser-check','--hide-scrollbars',`--user-data-dir=${profile}`,'--remote-debugging-address=127.0.0.1','--remote-debugging-port=0','about:blank'],{windowsHide:true,stdio:'ignore'});
 let ws;
 try{
  let port;
  for(let i=0;i<100;i++){try{port=(await fs.readFile(path.join(profile,'DevToolsActivePort'),'utf8')).split('\n')[0];break;}catch{await sleep(100);}}
  if(!port)throw Error('Edge did not expose its isolated DevTools port');
  const base=`http://127.0.0.1:${port}`;
  const protocol=await(await fetch(`${base}/json/protocol`)).json();
  if(!protocol.domains.find(d=>d.domain==='Emulation').commands.some(c=>c.name==='setEmulatedMedia'))throw Error('Installed Edge lacks media emulation');
  const version=await(await fetch(`${base}/json/version`)).json();
  const targets=await(await fetch(`${base}/json/list`)).json();const target=targets.find(t=>t.type==='page');
  ws=new WebSocket(target.webSocketDebuggerUrl);await new Promise((resolve,reject)=>{ws.addEventListener('open',resolve,{once:true});ws.addEventListener('error',reject,{once:true});});
  let next=0;const pending=new Map(),events=new Map(),requests=[];
  ws.addEventListener('message',event=>{const data=JSON.parse(event.data);if(data.id){const waiter=pending.get(data.id);if(waiter){pending.delete(data.id);data.error?waiter.reject(Error(JSON.stringify(data.error))):waiter.resolve(data.result);}}else{if(data.method==='Network.requestWillBeSent')requests.push(data.params.request.url);for(const callback of events.get(data.method)||[])callback(data.params);}});
  const cmd=(method,params={})=>new Promise((resolve,reject)=>{const id=++next;pending.set(id,{resolve,reject});ws.send(JSON.stringify({id,method,params}));setTimeout(()=>{if(pending.delete(id))reject(Error(`Timed out: ${method}`));},15000).unref();});
  await cmd('Page.enable');await cmd('Network.enable');
  await cmd('Emulation.setDeviceMetricsOverride',{width,height,deviceScaleFactor:1,mobile:width===390});
  await cmd('Emulation.setEmulatedMedia',{features:[{name:'prefers-color-scheme',value:theme}]});
  const loaded=new Promise(resolve=>events.set('Page.loadEventFired',[resolve]));await cmd('Page.navigate',{url:pathToFileURL(source).href});await loaded;
  await cmd('Runtime.evaluate',{expression:'new Promise(r=>requestAnimationFrame(()=>requestAnimationFrame(r)))',awaitPromise:true});
  const status=(await cmd('Runtime.evaluate',{expression:"({dark:matchMedia('(prefers-color-scheme: dark)').matches,background:getComputedStyle(document.body).backgroundColor,width:innerWidth,height:innerHeight,images:document.images.length,scripts:document.scripts.length})",returnByValue:true})).result.value;
  if(status.dark!==(theme==='dark')||status.width!==width||status.scripts!==0||status.images!==2)throw Error(JSON.stringify(status));
  if(requests.some(url=>/^https?:/i.test(url)))throw Error('The exported page made an external request');
  const screenshot=await cmd('Page.captureScreenshot',{format:'png',fromSurface:true,captureBeyondViewport:false,clip:{x:0,y:0,width,height,scale:1}});
  const png=path.join(out,`share-html-${width}-${theme}.png`);const bytes=Buffer.from(screenshot.data,'base64');if(bytes.readUInt32BE(16)!==width||bytes.readUInt32BE(20)!==height)throw Error('Wrong screenshot dimensions');await fs.writeFile(png,bytes);
  rows.push({browser:edge,browserVersion:version.Browser,headless:true,width,height,theme,png,computed:status,http_requests:0,media_emulation:'Emulation.setEmulatedMedia'});
  await cmd('Browser.close').catch(()=>{});
 }finally{
  if(ws)ws.close();await sleep(200);if(proc.exitCode===null){try{execFileSync('taskkill',['/PID',String(proc.pid),'/T','/F'],{windowsHide:true,stdio:'ignore'});}catch{}}
 }
}
await fs.writeFile(path.join(out,'edge-renders.json'),JSON.stringify(rows,null,2));console.log('PASS: isolated Edge headless media emulation; no scripts or external page requests; four PNGs.');
