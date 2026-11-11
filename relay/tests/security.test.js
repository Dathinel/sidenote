// CI-only tests. These mocks exercise the actual Worker/DO handlers without deployment.
import { test, expect, mock } from "bun:test";
import { Database } from "bun:sqlite";
mock.module("cloudflare:workers", () => ({ DurableObject: class {
  constructor(ctx, env) { this.ctx = ctx; this.env = env; }
}}));
globalThis.WebSocketRequestResponsePair = class {};
globalThis.WebSocket = { OPEN: 1 };
const { default: worker, Room } = await import("../src/index.ts");
const { PAIR_LOCATORS } = await import("../src/locators.ts");

function fixture(overrides) {
  let env;
  const budgets = new Map();
  env = {ROOMS:{getByName(name) {
    if(!budgets.has(name)) budgets.set(name,fixture(env).room);
    return {fetch:request=>budgets.get(name).fetch(new Request(request))};
  }},...overrides};
  let db = new Database(":memory:");
  const sockets = [];
  let deletes = 0, writes = 0, alarm = null;
  const ctx = {
    storage: {
      sql: { exec(query, ...values) {
        if (/^(INSERT|UPDATE|DELETE)/.test(query)) writes++;
        const rows = db.query(query).all(...values);
        return { toArray: () => rows };
      }},
      getAlarm: async () => alarm,
      setAlarm: async value => { alarm = value; },
      deleteAll: async () => { db.close(); db = new Database(":memory:"); deletes++; },
    },
    getWebSockets: () => sockets,
    acceptWebSocket() {},
    setWebSocketAutoResponse() {},
  };
  const room = new Room(ctx, env);
  function socket(state) {
    const ws = { readyState: 1, closed: false, sent: [],
      deserializeAttachment: () => state,
      serializeAttachment: value => { state = value; },
      send: value => ws.sent.push(value),
      close: () => { ws.closed = true; ws.readyState = 2; },
    };
    sockets.push(ws); return ws;
  }
  return { room, socket, sockets, get writes() { return writes; }, get deletes() { return deletes; }, get alarm() { return alarm; } };
}
const key = "a".repeat(64);
const state = role => ({room:key, kind:"pair", role, key, nonce:"", deadline:Date.now()+600_000, window:Date.now(), messages:0, bytes:0});
const pair = () => ({kind:"pair", pins:[key], expires:Date.now()+600_000, attempts:0, used:false, retries:0, messages:0, grantForwarded:false, receiptForwarded:false, cleanupAt:0});

test("expired invitation rejects a guest before a socket or bundle exists", async () => {
  const f=fixture(), meta=pair();meta.expires=Date.now()-1;f.room.save(meta);
  const response=await f.room.fetch(new Request(`http://127.0.0.1/pair/${key}?role=guest`,{headers:{Upgrade:"websocket"}}));
  expect(response.status).toBe(410);
  expect(f.sockets.length).toBe(0);
  expect(meta.grantForwarded).toBe(false);
});

test("R1 only the 1000 public locator hashes reach a DO", async () => {
  expect(PAIR_LOCATORS.size).toBe(1000);
  let calls=0;
  const env={ROOMS:{getByName(){ calls++; return {fetch:async()=>new Response("ok")}; }}};
  const request=name=>new Request(`https://relay.test/pair/${name}?role=owner`,{headers:{Upgrade:"websocket"}});
  expect((await worker.fetch(request("f".repeat(64)),env)).status).toBe(404);
  expect(calls).toBe(0);
  await worker.fetch(request([...PAIR_LOCATORS][0]),env);
  expect(calls).toBe(2);
});
test("R1 strict device login budget survives handler turns", async () => {
  const f=fixture();
  for(let i=0;i<10;i++) expect(await f.room.admitLogin()).toBe(true);
  expect(await f.room.admitLogin()).toBe(false);
});
test("R2 fourth retry and sixty-fifth authenticated pair message close the room", async () => {
  const f=fixture(), meta=pair();f.room.save(meta);
  const owner=f.socket(state("owner"));
  for(let i=0;i<4;i++) await f.room.webSocketMessage(owner,'{"type":"retry"}');
  expect(owner.closed).toBe(true);
  const g=fixture();g.room.save({...pair(),messages:64});
  const ws=g.socket(state("owner"));await g.room.webSocketMessage(ws,new ArrayBuffer(33));
  expect(ws.closed).toBe(true);
});
test("R2 a frame for an absent guest leaves the owner open", async () => {
  const f=fixture();f.room.save(pair());const owner=f.socket(state("owner"));
  await f.room.webSocketMessage(owner,new ArrayBuffer(33));expect(owner.closed).toBe(false);
});
test("F6 oldest unauthenticated socket is evicted to admit a new one", async () => {
  const f=fixture();f.room.save(pair());
  const old=[];
  for(let i=0;i<4;i++) old.push(f.socket({...state("guest"),key:undefined,deadline:Date.now()+1000+i}));
  const NativeResponse=globalThis.Response, NativePair=globalThis.WebSocketPair;
  globalThis.WebSocketPair=class { constructor() {this[0]={}; this[1]=f.socket({...state("guest"),key:undefined});} };
  globalThis.Response=class extends NativeResponse {constructor(body,init) {if(init?.status===101) return {status:101};super(body,init);} };
  try {
    const response=await f.room.fetch(new Request(`https://relay.test/pair/${key}?role=guest`));
    expect(response.status).toBe(101);
    expect(old[0].closed).toBe(true);
    expect(old.slice(1).every(ws=>!ws.closed)).toBe(true);
  } finally {globalThis.Response=NativeResponse;globalThis.WebSocketPair=NativePair;}
});

for (const kind of ["pair", "sync"]) {
  test(`F6 ${kind} retained sockets stay capped at 32 when evicted peers never finish closing`, async () => {
    const f=fixture(), role=kind==="pair" ? "guest" : "device";
    if(kind==="pair") f.room.save(pair());
    const NativeResponse=globalThis.Response, NativePair=globalThis.WebSocketPair;
    globalThis.WebSocketPair=class { constructor() {this[0]={}; this[1]=f.socket({...state(role),kind,key:undefined});} };
    globalThis.Response=class extends NativeResponse {constructor(body,init) {if(init?.status===101) return {status:101};super(body,init);} };
    try {
      for(let i=0;i<64;i++) {
        const response=await f.room.fetch(new Request(`https://relay.test/${kind}/${key}?role=${role}`));
        expect(f.sockets.length).toBeLessThanOrEqual(32);
        expect(response.status).toBe(i<32 ? 101 : 429);
      }
      expect(f.sockets.length).toBe(32);
      expect(f.sockets.some(ws=>ws.closed && ws.readyState===2)).toBe(true);
    } finally {globalThis.Response=NativeResponse;globalThis.WebSocketPair=NativePair;}
  });
}

test("F6 three full pairing attempts fit the authenticated frame budget",async()=>{
  const f=fixture(); f.room.save(pair()); const owner=f.socket(state("owner"));
  for(let attempt=1;attempt<=3;attempt++) {
    const guest=f.socket({...state("guest"),key:"b".repeat(64)});
    f.room.save({...f.room.meta(),attempts:attempt});
    // Both PAKE points, both identity proofs, both confirmation frames.
    for(let frame=0;frame<3;frame++) {
      await f.room.webSocketMessage(owner,new ArrayBuffer(100));
      await f.room.webSocketMessage(guest,new ArrayBuffer(100));
    }
    if(attempt<3) await f.room.webSocketMessage(owner,'{"type":"retry"}');
    else {
      await f.room.webSocketMessage(owner,'{"type":"grant"}');
      await f.room.webSocketMessage(owner,new ArrayBuffer(100));
      await f.room.webSocketMessage(guest,new ArrayBuffer(100));
      expect(f.room.meta().receiptForwarded).toBe(true);
    }
    expect(owner.closed).toBe(false);
  }
  expect(f.room.meta().messages).toBe(23);
});

test("F6 unauthenticated frames cannot consume the pairing room budget",async()=>{
  const f=fixture();f.room.save(pair());const owner=f.socket(state("owner"));
  for(let i=0;i<80;i++) {
    const stranger=f.socket({...state("guest"),key:undefined});
    await f.room.webSocketMessage(stranger,new ArrayBuffer(100));
    expect(stranger.closed).toBe(true);
  }
  expect(f.room.meta().messages).toBe(0);
  expect(owner.closed).toBe(false);
});

test("R3 expired and consumed pairing data is deleted by alarm", async () => {
  for(const meta of [{...pair(),expires:0},{...pair(),used:true,cleanupAt:0}]) {
    const f=fixture();f.room.save(meta);await f.room.alarm();expect(f.deletes).toBe(1);expect(f.room.meta()).toBeUndefined();
  }
});
test("R4 oversized text is refused before UTF8 encoding", async () => {
  const f=fixture();f.room.save(pair());const ws=f.socket(state("owner"));
  const encoder=TextEncoder.prototype.encode;
  let oversizedEncodes=0;
  TextEncoder.prototype.encode=function(value) {if(value.length>128*1024) oversizedEncodes++;return encoder.call(this,value);};
  try {await f.room.webSocketMessage(ws,"x".repeat(128*1024+1));}
  finally {TextEncoder.prototype.encode=encoder;}
  expect(ws.closed).toBe(true);expect(oversizedEncodes).toBe(0);
});
test("R5 grant consumes before exactly one bundle; retry cannot reopen it", async () => {
  const f=fixture();f.room.save(pair());const owner=f.socket(state("owner"));f.socket({...state("guest"),key:"b".repeat(64)});
  await f.room.webSocketMessage(owner,'{"type":"grant"}');expect(f.room.meta().used).toBe(true);
  await f.room.webSocketMessage(owner,new ArrayBuffer(100));expect(f.room.meta().grantForwarded).toBe(true);
  await f.room.webSocketMessage(owner,'{"type":"retry"}');
  expect(owner.closed).toBe(true);expect(f.room.meta().used).toBe(true);expect(f.room.meta().retries).toBe(0);
  const duplicate=fixture();duplicate.room.save({...pair(),used:true,grantForwarded:true});
  const owner2=duplicate.socket(state("owner"));duplicate.socket({...state("guest"),key:"b".repeat(64)});
  await duplicate.room.webSocketMessage(owner2,new ArrayBuffer(100));expect(owner2.closed).toBe(true);
});

const hex = bytes => Array.from(new Uint8Array(bytes), b=>b.toString(16).padStart(2,"0")).join("");
async function signedLogin(invalidMember = false, role = "device") {
  const keys=await crypto.subtle.generateKey("Ed25519",true,["sign","verify"]);
  const publicKey=hex(await crypto.subtle.exportKey("raw",keys.publicKey));
  const sign=async text=>hex(await crypto.subtle.sign("Ed25519",keys.privateKey,new TextEncoder().encode(text)));
  const attachment={...state(role),kind:role==="owner"?"pair":"sync",nonce:"c".repeat(64),key:undefined};
  const enrollments=[{sponsor:publicKey,member:publicKey,signature:await sign(`sidenote-enroll-v1:${key}:${publicKey}`)}];
  if(invalidMember) {
    const member="q".repeat(64);
    enrollments.push({sponsor:publicKey,member,signature:await sign(`sidenote-enroll-v1:${key}:${member}`)});
  }
  return {attachment,auth:JSON.stringify({type:"auth",key:publicKey,signature:await sign(`sidenote-auth-v1:${key}:${role}:${attachment.nonce}`),enrollments})};
}
test("R4 even a valid sponsor signature cannot enroll a malformed member key",async()=>{
  const f=fixture(),{attachment,auth}=await signedLogin(true),ws=f.socket(attachment);
  await f.room.webSocketMessage(ws,auth);
  expect(ws.closed).toBe(true);expect(f.room.meta()).toBeUndefined();
});
test("R3 unchanged sync pins do not rewrite room metadata",async()=>{
  const f=fixture(),{attachment,auth}=await signedLogin();
  await f.room.webSocketMessage(f.socket({...attachment}),auth);
  const before=f.writes;
  await f.room.webSocketMessage(f.socket({...attachment}),auth);
  expect(f.writes).toBe(before);expect(f.room.meta().pins.length).toBe(1);
});

test("inactive self-created sync rooms expire and authenticated activity renews them",async()=>{
  const ttl=24*60*60*1000;
  const f=fixture(),{attachment,auth}=await signedLogin();
  const ws=f.socket({...attachment});
  const before=Date.now();
  await f.room.webSocketMessage(ws,auth);
  expect(f.room.meta().expires).toBeGreaterThanOrEqual(before+ttl);
  expect(f.alarm).toBe(f.room.meta().expires);
  f.room.save({...f.room.meta(),expires:Date.now()+1000});
  await f.room.webSocketMessage(ws,'{"type":"ping"}');
  expect(f.room.meta().expires).toBeGreaterThanOrEqual(before+ttl);
  const unchanged=f.writes;
  await f.room.webSocketMessage(ws,'{"type":"ping"}');
  expect(f.writes).toBe(unchanged);
  f.room.save({...f.room.meta(),expires:Date.now()-1});
  await f.room.alarm();
  expect(f.deletes).toBe(1);expect(f.room.meta()).toBeUndefined();expect(ws.closed).toBe(true);
});

test("unauthenticated traffic cannot renew an expired sync room",async()=>{
  const f=fixture();
  const expired={...pair(),kind:"sync",expires:Date.now()-1};
  f.room.save(expired);
  const stranger=f.socket({...state("device"),kind:"sync",key:undefined});
  await f.room.webSocketMessage(stranger,new ArrayBuffer(1));
  expect(f.room.meta().expires).toBe(expired.expires);
  await f.room.alarm();
  expect(f.deletes).toBe(1);expect(f.room.meta()).toBeUndefined();
});

test("F8 authenticated logins consume the real cross-room device budget",async()=>{
  const f=fixture(),{attachment,auth}=await signedLogin();
  for(let i=0;i<10;i++) {
    const ws=f.socket({...attachment});await f.room.webSocketMessage(ws,auth);
    expect(ws.closed).toBe(false);
    expect(ws.sent.some(text=>JSON.parse(text).type==="ready")).toBe(true);
  }
  const blocked=f.socket({...attachment});await f.room.webSocketMessage(blocked,auth);
  expect(blocked.closed).toBe(true);
  expect(blocked.sent).toEqual([]);
});

test("local proof routes only the quota burst to the production room",async()=>{
  const {default:proof}=await import("../harness/observed.ts");
  const namespace=label=>({getByName:()=>({fetch:async()=>new Response(label)})});
  const env={ROOMS:namespace("observed"),RATE_ROOMS:namespace("production")};
  for(const [query,expected] of [["","observed"],["&__proof_rate=1","production"]]) {
    const request=new Request(`https://relay.test/sync/${key}?role=device${query}`,{headers:{Upgrade:"websocket"}});
    expect(await (await proof.fetch(request,env)).text()).toBe(expected);
  }
});

test("sync quota rejects message 601 and resets after a full second",async()=>{
  const originalNow=Date.now;
  let now=1000;
  Date.now=()=>now;
  try {
    const f=fixture();
    f.room.save({...pair(),kind:"sync",expires:now+24*60*60*1000});
    const ws=f.socket({...state("device"),kind:"sync"});
    for(let i=0;i<600;i++) {
      await f.room.webSocketMessage(ws,'{"type":"ping"}');
      expect(ws.closed).toBe(false);
    }
    await f.room.webSocketMessage(ws,'{"type":"ping"}');
    expect(ws.closed).toBe(true);
    const next=f.socket({...state("device"),kind:"sync",messages:600});
    now+=1000;
    await f.room.webSocketMessage(next,'{"type":"ping"}');
    expect(next.closed).toBe(false);
    expect(next.deserializeAttachment().messages).toBe(1);
  } finally {Date.now=originalNow;}
});


test("production worker exposes no local expiry control", async () => {
  const { default: worker } = await import("../src/index.ts");
  let routed = false;
  const response = await worker.fetch(new Request("https://relay.test/__test/expire/" + "a".repeat(64), { method: "POST" }), {
    ROOMS: { getByName() { routed = true; throw Error("unexpected test route"); } },
  });
  expect(response.status).toBe(404);
  expect(routed).toBe(false);
});


test("health checks are public, bounded and do not allocate a room", async () => {
 let calls=0;const env={ROOMS:{getByName(){calls++;throw Error("must not allocate");}}};
 const get=await worker.fetch(new Request("http://127.0.0.1/health"),env);
 expect(get.status).toBe(200);expect(await get.text()).toBe("sidenote-relay-v1");expect(get.headers.get("Cache-Control")).toBe("no-store");
 const head=await worker.fetch(new Request("http://127.0.0.1/health",{method:"HEAD"}),env);expect(await head.text()).toBe("");
 expect((await worker.fetch(new Request("http://127.0.0.1/health",{method:"POST"}),env)).status).toBe(405);expect(calls).toBe(0);
});


test("oversized binary frame closes the sender without delivery or state changes",async()=>{
 const f=fixture();f.room.save({...pair(),kind:"sync"});const sender=f.socket({...state("device"),kind:"sync"});const recipient=f.socket({...state("device"),kind:"sync",key:"b".repeat(64)});
 const before=JSON.stringify(f.room.meta());await f.room.webSocketMessage(sender,new ArrayBuffer(128*1024+1));
 expect(sender.closed).toBe(true);expect(recipient.sent.length).toBe(0);expect(JSON.stringify(f.room.meta())).toBe(before);
});


test("production does not expose sync-interruption qualification controls",async()=>{
 let calls=0;const env={ROOMS:{getByName(){calls++;throw Error("unexpected room");}}};
 for(const mode of ["pause","resume","status"]){const response=await worker.fetch(new Request(`http://127.0.0.1/__test/sync-${mode}/${"a".repeat(64)}`,{method:"POST"}),env);expect(response.status).toBe(404);}
 expect(calls).toBe(0);
});

test("allowlist refuses unlisted genesis without persisting a sync room",async()=>{
  const f=fixture({ALLOWED_GENESIS:"f".repeat(64)}),{attachment,auth}=await signedLogin();
  const ws=f.socket(attachment);await f.room.webSocketMessage(ws,auth);
  expect(ws.closed).toBe(true);expect(f.room.meta()).toBeUndefined();expect(ws.sent).toEqual([]);
  const publicKey=JSON.parse(auth).key;
  const allowed=fixture({ALLOWED_GENESIS:publicKey}),good=allowed.socket({...attachment});
  await allowed.room.webSocketMessage(good,auth);
  expect(good.closed).toBe(false);expect(allowed.room.meta().pins).toEqual([publicKey]);
});

test("group allowlist refuses unknown sync rooms before object allocation",async()=>{
  let calls=0;
  const env={ALLOWED_ROOMS:key,ROOMS:{getByName(){calls++;return {fetch:async()=>new Response("accepted")};}}};
  const request=room=>new Request(`https://relay.test/sync/${room}?role=device`,{headers:{Upgrade:"websocket"}});
  expect((await worker.fetch(request("b".repeat(64)),env)).status).toBe(403);
  expect(calls).toBe(0);
  expect((await worker.fetch(request(key),env)).status).toBe(200);
  expect(calls).toBe(1);
});

test("pair-owner allowlist and three pending slots are enforced across rooms",async()=>{
  const {attachment,auth}=await signedLogin(false,"owner");
  const publicKey=JSON.parse(auth).key;
  const denied=fixture({ALLOWED_GENESIS:"f".repeat(64)});
  const ws=denied.socket({...attachment,kind:"pair",role:"owner"});
  await denied.room.webSocketMessage(ws,auth);expect(ws.closed).toBe(true);
  expect(denied.room.meta()).toBeUndefined();
  const allowed=fixture({ALLOWED_GENESIS:publicKey}),owner=allowed.socket({...attachment});
  await allowed.room.webSocketMessage(owner,auth);
  expect(owner.closed).toBe(false);expect(allowed.room.meta().pins).toEqual([publicKey]);
  const budget=fixture();
  const claim=(room,release=false)=>budget.room.fetch(new Request(`https://budget.internal/pair-budget?room=${room}&limit=3&release=${release?1:0}`));
  const rooms=[1,2,3,4].map(i=>i.toString(16).padStart(64,"0"));
  for(const room of rooms.slice(0,3))expect((await claim(room)).status).toBe(204);
  expect((await claim(rooms[0])).status).toBe(204);
  expect((await claim(rooms[3])).status).toBe(429);
  expect((await claim(rooms[0],true)).status).toBe(204);
  expect((await claim(rooms[3])).status).toBe(204);
  expect(publicKey.length).toBe(64);
});

test("IP pairing cap refuses a ninth pending room before room allocation",async()=>{
  const router=fixture();let allocated=0;
  const env={ROOMS:{getByName(name){
    if(name.startsWith("pair-budget:"))return {fetch:request=>router.room.fetch(new Request(request))};
    allocated++;return {fetch:async()=>new Response("accepted")};
  }}};
  const rooms=[...PAIR_LOCATORS].slice(0,9);
  for(const room of rooms.slice(0,8))expect((await worker.fetch(new Request(`https://relay.test/pair/${room}?role=owner`,{headers:{Upgrade:"websocket","CF-Connecting-IP":"192.0.2.10"}}),env)).status).toBe(200);
  expect((await worker.fetch(new Request(`https://relay.test/pair/${rooms[8]}?role=owner`,{headers:{Upgrade:"websocket","CF-Connecting-IP":"192.0.2.10"}}),env)).status).toBe(429);
  expect(allocated).toBe(8);
});

test("public entry point cannot change internal pairing budgets",async()=>{
  let calls=0;
  const env={ROOMS:{getByName(){calls++;throw Error("unexpected allocation");}}};
  for(const action of ["0","1"]) {
    const response=await worker.fetch(new Request(`https://relay.test/pair-budget?room=${key}&limit=3&release=${action}`),env);
    expect(response.status).toBe(404);
  }
  expect(calls).toBe(0);
});

test("connection byte budget refuses more than four MiB per second",async()=>{
  const f=fixture();f.room.save({...pair(),kind:"sync",pins:[key,"b".repeat(64)]});
  f.room.rateNow=()=>1000;
  const ws=f.socket({...state("device"),kind:"sync"}),peer=f.socket({...state("device"),kind:"sync",key:"b".repeat(64)});
  ws.serializeAttachment({...ws.deserializeAttachment(),window:1000});
  const bytes=new Uint8Array(128*1024);bytes[0]=1;bytes[1]=3;
  bytes.fill(0xaa,2,34);bytes.fill(0xbb,34,66);
  for(let i=0;i<32;i++)await f.room.webSocketMessage(ws,bytes.buffer);
  expect(ws.closed).toBe(false);expect(peer.sent.length).toBe(32);
  await f.room.webSocketMessage(ws,bytes.buffer);
  expect(ws.closed).toBe(true);expect(peer.sent.length).toBe(32);
});
