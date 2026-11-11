import { DurableObject } from "cloudflare:workers";
import { PAIR_LOCATORS } from "./locators";

const MAX_FRAME = 128 * 1024;
const PAIR_MESSAGES = 64;
const PAIR_TTL = 10 * 60 * 1000;
const SYNC_TTL = 24 * 60 * 60 * 1000;
type Kind = "pair" | "sync";
type Role = "owner" | "guest" | "device";
type Meta = { kind: Kind; pins: string[]; expires: number; attempts: number; used: boolean; retries: number; messages: number; grantForwarded: boolean; receiptForwarded: boolean; cleanupAt: number };
type Attachment = {
  room: string; kind: Kind; role: Role; nonce: string; key?: string;
  deadline: number; window: number; messages: number; bytes: number; pairIp?: string;
};
type Enrollment = { sponsor: string; member: string; signature: string };
const enc = new TextEncoder();
const hex = (bytes: Uint8Array) => Array.from(bytes, b => b.toString(16).padStart(2, "0")).join("");
function unhex(s: string): Uint8Array<ArrayBuffer> {
  if (!/^(?:[a-f0-9]{2})+$/.test(s)) throw Error("invalid encoding");
  return Uint8Array.from(s.match(/../g)!, h => parseInt(h, 16));
}
const validKey = (s: unknown): s is string => typeof s === "string" && /^[a-f0-9]{64}$/.test(s);
function allowedGenesis(env: Env, key: string): boolean {
  const entries = (env.ALLOWED_GENESIS ?? "").trim();
  return !entries || entries.split(",").map(value => value.trim()).includes(key);
}
async function pairBudget(env: Env, principal: string, room: string, limit: number, release = false): Promise<boolean> {
  return (await env.ROOMS.getByName(`pair-budget:${principal}`).fetch(`https://budget.internal/pair-budget?room=${room}&limit=${limit}&release=${release ? 1 : 0}`)).ok;
}
async function verify(key: string, message: string, signature: string) {
  if (!validKey(key) || !/^[a-f0-9]{128}$/.test(signature)) throw Error("invalid proof");
  const imported = await crypto.subtle.importKey("raw", unhex(key), "Ed25519", false, ["verify"]);
  if (!await crypto.subtle.verify("Ed25519", imported, unhex(signature), enc.encode(message))) throw Error("invalid proof");
}

export default {
  async fetch(request: Request, env: Env): Promise<Response> {
    const url = new URL(request.url);
    if (url.pathname === "/health") {
      if (request.method !== "GET" && request.method !== "HEAD") return new Response("method not allowed", {status:405,headers:{Allow:"GET, HEAD"}});
      return new Response(request.method === "HEAD" ? null : "sidenote-relay-v1", {headers:{"Content-Type":"text/plain; charset=utf-8","Cache-Control":"no-store"}});
    }
    const match = /^\/(pair|sync)\/([a-f0-9]{64})$/.exec(url.pathname);
    if (!match || request.method !== "GET") return new Response("not found", { status: 404 });
    if (request.headers.get("Upgrade")?.toLowerCase() !== "websocket") return new Response("WebSocket required", { status: 426 });
    const allowedRooms=(env.ALLOWED_ROOMS ?? "").trim();
    if (match[1] === "sync" && allowedRooms && !allowedRooms.split(",").map(value=>value.trim()).includes(match[2])) return new Response("group not allowed",{status:403});
    if (match[1] === "pair" && !PAIR_LOCATORS.has(match[2])) return new Response("invalid locator", { status: 404 });
    if (match[1] === "pair" && url.searchParams.get("role") === "owner") {
      const ip = request.headers.get("CF-Connecting-IP") ?? "unknown";
      const principal = hex(new Uint8Array(await crypto.subtle.digest("SHA-256", enc.encode(ip))));
      if (!await pairBudget(env, `ip:${principal}`, match[2], 8)) return new Response("pairing capacity reached", {status:429});
    }
    return env.ROOMS.getByName(`${match[1]}:${match[2]}`).fetch(request);
  },
} satisfies ExportedHandler<Env>;

export class Room extends DurableObject<Env> {
  protected observe(_direction: "in" | "out", _kind: string, _bytes: Uint8Array): void {}
  protected rateNow(): number { return Date.now(); }
  private schema = false;
  private ensureSchema() {
    if (this.schema) return;
    this.ctx.storage.sql.exec("CREATE TABLE IF NOT EXISTS metadata (id INTEGER PRIMARY KEY CHECK(id=1), value TEXT NOT NULL)");
    this.ctx.storage.sql.exec("CREATE TABLE IF NOT EXISTS login_budget (id INTEGER PRIMARY KEY CHECK(id=1), window INTEGER NOT NULL, count INTEGER NOT NULL)");
    this.ctx.storage.sql.exec("CREATE TABLE IF NOT EXISTS pair_claims (room TEXT PRIMARY KEY, expires INTEGER NOT NULL)");
    this.schema = true;
  }
  async admitLogin(): Promise<boolean> {
    this.ensureSchema();
    const now = Date.now();
    const row = this.ctx.storage.sql.exec<{window: number; count: number}>("SELECT window,count FROM login_budget WHERE id=1").toArray()[0];
    const window = row && now < row.window + 60_000 ? row.window : now;
    const count = row && window === row.window ? row.count : 0;
    if (count >= 10) return false;
    this.ctx.storage.sql.exec("INSERT INTO login_budget VALUES(1,?,?) ON CONFLICT(id) DO UPDATE SET window=excluded.window,count=excluded.count", window, count+1);
    await this.schedule(window + 60_000);
    return true;
  }
  constructor(ctx: DurableObjectState, env: Env) {
    super(ctx, env);
    this.ensureSchema();
    this.ctx.setWebSocketAutoResponse(new WebSocketRequestResponsePair('{"type":"ping"}', '{"type":"pong"}'));
  }
  private meta(): Meta | undefined {
    this.ensureSchema();
    const row = this.ctx.storage.sql.exec<{ value: string }>("SELECT value FROM metadata WHERE id=1").toArray()[0];
    return row ? JSON.parse(row.value) as Meta : undefined;
  }
  private save(meta: Meta) {
    this.ctx.storage.sql.exec("INSERT INTO metadata VALUES(1, ?) ON CONFLICT(id) DO UPDATE SET value=excluded.value", JSON.stringify(meta));
  }
  private async touchSync(meta: Meta) {
    // Coalesce activity renewals so normal sync does not write SQL for every frame.
    if (meta.expires <= Date.now() + SYNC_TTL / 2) {
      meta.expires = Date.now() + SYNC_TTL;
      this.save(meta);
    }
    await this.schedule(meta.expires);
  }
  private state(ws: WebSocket): Attachment { return ws.deserializeAttachment() as Attachment; }
  private sockets(role?: Role): WebSocket[] {
    return this.ctx.getWebSockets().filter(ws => {
      const a = this.state(ws);
      return a.key && ws.readyState === WebSocket.OPEN && (!role || a.role === role);
    });
  }
  private send(ws: WebSocket, value: object) { const text=JSON.stringify(value);this.observe("out","control",enc.encode(text));ws.send(text); }
  private close(ws: WebSocket,code: number,reason: string) {this.observe("out","close",enc.encode(reason));ws.close(code,reason);}
  private closeAll(reason: string) { for (const ws of this.ctx.getWebSockets()) this.close(ws,1008, reason); }
  private async schedule(deadline: number) {
    const current = await this.ctx.storage.getAlarm();
    if (current === null || deadline < current) await this.ctx.storage.setAlarm(deadline);
  }
  async fetch(request: Request): Promise<Response> {
    const url = new URL(request.url);
    if (url.pathname === "/pair-budget") {
      const room = url.searchParams.get("room"), limit = Number(url.searchParams.get("limit"));
      if (!validKey(room) || ![3,8].includes(limit)) return new Response(null,{status:400});
      this.ensureSchema();
      this.ctx.storage.sql.exec("DELETE FROM pair_claims WHERE expires<=?",Date.now());
      if (url.searchParams.get("release") === "1") {
        this.ctx.storage.sql.exec("DELETE FROM pair_claims WHERE room=?",room);
        return new Response(null,{status:204});
      }
      const existing=this.ctx.storage.sql.exec("SELECT room FROM pair_claims WHERE room=?",room).toArray();
      const count=this.ctx.storage.sql.exec<{count:number}>("SELECT COUNT(*) AS count FROM pair_claims").toArray()[0].count;
      if (!existing.length && count>=limit) return new Response(null,{status:429});
      this.ctx.storage.sql.exec("INSERT OR IGNORE INTO pair_claims VALUES(?,?)",room,Date.now()+PAIR_TTL);
      await this.schedule(Date.now()+PAIR_TTL);
      return new Response(null,{status:204});
    }
    if (url.pathname === "/login-budget") return new Response(null,{status:await this.admitLogin() ? 204 : 429});
    const [, kind, room] = url.pathname.split("/") as [string, Kind, string];
    const role = url.searchParams.get("role") as Role;
    if ((kind === "pair" && role !== "owner" && role !== "guest") || (kind === "sync" && role !== "device")) return new Response("invalid role", { status: 400 });
    let meta = this.meta();
    if (kind === "pair" && (!meta || (meta.expires <= Date.now() && role === "owner"))) {
      if (meta) this.closeAll("previous invitation expired");
      meta = {kind:"pair",pins:[],expires:Date.now()+PAIR_TTL,attempts:0,used:false,retries:0,messages:0,grantForwarded:false,receiptForwarded:false,cleanupAt:0};
      this.save(meta);
    }
    if (kind === "pair" && meta && ((meta.expires > Date.now() && (meta.used || meta.attempts >= 3)) || (meta.expires <= Date.now() && role !== "owner"))) return new Response("invitation closed", { status: 410 });
    const unauthenticated = this.ctx.getWebSockets().filter(ws => !this.state(ws).key && ws.readyState === WebSocket.OPEN);
    if (unauthenticated.length >= 4) {
      const oldest = unauthenticated.reduce((a,b) => this.state(a).deadline <= this.state(b).deadline ? a : b);
      this.close(oldest,1008,"unauthenticated socket replaced");
    }
    if (this.ctx.getWebSockets().length >= 32) return new Response("room full", { status: 429 });
    const pair = new WebSocketPair();
    const [client, server] = [pair[0], pair[1]];
    const nonce = hex(crypto.getRandomValues(new Uint8Array(32)));
    const deadline = Date.now() + 30_000;
    this.ctx.acceptWebSocket(server);
    const ip = request.headers.get("CF-Connecting-IP") ?? "unknown";
    const pairIp = kind === "pair" && role === "owner" ? hex(new Uint8Array(await crypto.subtle.digest("SHA-256", enc.encode(ip)))) : undefined;
    server.serializeAttachment({ room, kind, role, nonce, deadline, window: this.rateNow(), messages: 0, bytes: 0, pairIp } satisfies Attachment);
    this.send(server, { type: "challenge", nonce });
    await this.schedule(deadline);
    return new Response(null, { status: 101, webSocket: client });
  }
  private rate(ws: WebSocket, size: number): Attachment {
    const a = this.state(ws), now = this.rateNow();
    if (a.kind === "pair" && a.key) {
      const meta=this.meta();
      if (!meta || ++meta.messages > PAIR_MESSAGES) {this.closeAll("pair message budget exhausted");throw Error("pair budget");}
      this.save(meta);
    }
    if (size > MAX_FRAME) throw Error("frame too large");
    if (now - a.window >= 1000) { a.window = now; a.messages = 0; a.bytes = 0; }
    a.messages++; a.bytes += size;
    if (a.messages > 600 || a.bytes > 4 * 1024 * 1024) throw Error("rate exceeded");
    ws.serializeAttachment(a);
    return a;
  }
  private async authenticate(ws: WebSocket, a: Attachment, text: string) {
    if (Date.now() > a.deadline) throw Error("authentication expired");
    const msg = JSON.parse(text) as { type: string; key: string; signature: string; enrollments?: Enrollment[] };
    if (msg.type !== "auth") throw Error("authentication required");
    await verify(msg.key, `sidenote-auth-v1:${a.room}:${a.role}:${a.nonce}`, msg.signature);
    let meta = this.meta();
    if (a.kind === "pair") {
      if (a.role === "owner") {
        if (!allowedGenesis(this.env,msg.key)) throw Error("owner not allowed");
        if (meta && meta.expires > Date.now() && meta.pins.length) throw Error("invitation occupied");
        if (!await pairBudget(this.env,`device:${msg.key}`,a.room,3)) throw Error("pairing capacity reached");
        meta=this.meta();
        if (meta && meta.expires > Date.now() && meta.pins.length) throw Error("invitation occupied");
        for (const old of this.ctx.getWebSockets()) if (old !== ws) this.close(old,1008, "previous invitation expired");
        meta = { kind: "pair", pins: [msg.key], expires: Date.now() + PAIR_TTL, attempts: 0, used: false, retries:0, messages:meta?.messages ?? 0, grantForwarded:false, receiptForwarded:false, cleanupAt:0 };
      } else {
        if (!meta || meta.used || meta.expires <= Date.now() || meta.attempts >= 3 || !this.sockets("owner").length || this.sockets("guest").length) throw Error("invitation unavailable");
        meta.attempts++;
      }
      this.save(meta);
      a.deadline = meta.expires;
      a.key = msg.key;
      ws.serializeAttachment(a);
      await this.schedule(meta.expires);
    } else {
      const enrollments = msg.enrollments;
      if (!enrollments || enrollments.length < 1 || enrollments.length > 16) throw Error("invalid membership");
      const known: string[] = [];
      for (const e of enrollments) {
        if (!validKey(e.member)) throw Error("invalid member");
        if ((!known.length && e.sponsor !== e.member) || (known.length && !known.includes(e.sponsor)) || known.includes(e.member)) throw Error("invalid enrollment chain");
        await verify(e.sponsor, `sidenote-enroll-v1:${a.room}:${e.member}`, e.signature);
        known.push(e.member);
      }
      // Crypto yields to other events; re-read before the synchronous authority update.
      if (!allowedGenesis(this.env,known[0])) throw Error("genesis not allowed");
      meta = this.meta();
      if (meta && (meta.kind !== "sync" || meta.pins[0] !== known[0])) throw Error("different genesis");
      if (!known.includes(msg.key)) throw Error("unregistered device");
      const pins = Array.from(new Set([...(meta?.pins ?? []), ...known]));
      if (pins.length > 16) throw Error("group full");
      if (!meta || pins.length !== meta.pins.length) {
        meta = { kind: "sync", pins, expires: Date.now() + SYNC_TTL, attempts: 0, used: false, retries:0, messages:0, grantForwarded:false, receiptForwarded:false, cleanupAt:0 };
        this.save(meta);
      }
      await this.touchSync(meta);
      if (!(await this.env.ROOMS.getByName(`login:${msg.key}`).fetch("https://budget.internal/login-budget")).ok) throw Error("login budget exceeded");
      a.deadline = 0;
    }
    // Replace only after fresh proof of the same private key.
    for (const old of this.sockets()) if (old !== ws && this.state(old).key === msg.key) this.close(old,1000, "new authenticated session");
    a.key = msg.key;
    a.nonce = "";
    ws.serializeAttachment(a);
    this.send(ws, { type: "ready", peers: this.sockets().filter(other => other !== ws).map(other => this.state(other).key) });
    for (const other of this.sockets()) if (other !== ws) this.send(other, { type: "joined", key: a.key });
  }
  async webSocketMessage(ws: WebSocket, message: string | ArrayBuffer) {
    try {
      if (typeof message === "string" && message.length > MAX_FRAME) throw Error("frame too large");
      const size = typeof message === "string" ? enc.encode(message).length : message.byteLength;
      const raw=typeof message === "string" ? enc.encode(message) : new Uint8Array(message);
      this.observe("in",typeof message === "string" ? "auth-or-control" : raw[0]===1 && raw.length>=162 ? `envelope-${raw[1]}` : "pair-binary",raw);
      const a = this.rate(ws, size);
      if (!a.key) {
        if (typeof message !== "string" || size > 16_384) throw Error("invalid auth");
        await this.authenticate(ws, a, message);
        return;
      }
      const meta = this.meta();
      if (!meta || (a.kind === "pair" && (meta.expires <= Date.now()))) throw Error("room closed");
      if (a.kind === "sync") await this.touchSync(meta);
      if (typeof message === "string") {
        const msg = JSON.parse(message) as { type: string };
        if (msg.type === "ping") {this.send(ws,{type:"pong"});return;}
        if (a.kind !== "pair" || a.role !== "owner") throw Error("binary frames required");
        if (msg.type === "retry") {
          if (meta.used || ++meta.retries > 3) {this.closeAll("retry budget exhausted");throw Error("retry budget");}
          this.save(meta);
          for (const guest of this.sockets("guest")) this.close(guest,1008, "pairing failed");
          if (meta.attempts >= 3) { meta.used = true; this.save(meta); this.closeAll("three attempts exhausted"); }
          else this.send(ws, { type: "retry-ready" });
        } else if (msg.type === "grant") {
          if (meta.used || !this.sockets("guest").length) throw Error("grant unavailable");
          meta.used=true; meta.cleanupAt=Date.now()+30_000; this.save(meta);
          await this.schedule(meta.cleanupAt);
          this.send(ws,{type:"grant-ready"});
        } else if (msg.type === "complete") {
          if (!meta.used) throw Error("grant required");
          meta.cleanupAt=Date.now();this.save(meta);
          await this.releasePair(a);
          this.closeAll("invitation consumed");
          await this.schedule(meta.cleanupAt);
        } else throw Error("invalid control");
        return;
      }
      if (a.kind === "pair") {
        if (size > 16_384) throw Error("invalid pairing frame");
        if (!this.sockets("guest").length) return;
        if (meta.used) {
          if (a.role === "owner") {if (meta.grantForwarded) throw Error("bundle already released");meta.grantForwarded=true;}
          else {if (!meta.grantForwarded || meta.receiptForwarded) throw Error("invalid grant receipt");meta.receiptForwarded=true;}
          this.save(meta);
        }
        for (const other of this.sockets()) if (other !== ws) {this.observe("out","pair-binary",new Uint8Array(message));other.send(message);}
        return;
      }
      const frame = new Uint8Array(message);
      // v1, kind, sender(32), recipient(32), session(32), opaque Noise, signature(64).
      if (size < 162 || frame[0] !== 1 || ![1, 2, 3, 4, 5].includes(frame[1]) || hex(frame.subarray(2, 34)) !== a.key) throw Error("invalid envelope");
      const recipient = hex(frame.subarray(34, 66));
      if (!meta.pins.includes(recipient) || recipient === a.key) throw Error("unknown recipient");
      for (const other of this.sockets()) if (this.state(other).key === recipient) {this.observe("out",`envelope-${frame[1]}`,frame);other.send(message);}
    } catch {
      this.close(ws,1008, "protocol rejected");
    }
  }
  private async releasePair(a: Attachment) {
    if (a.kind !== "pair" || a.role !== "owner") return;
    if (a.key) await pairBudget(this.env,`device:${a.key}`,a.room,3,true);
    if (a.pairIp) await pairBudget(this.env,`ip:${a.pairIp}`,a.room,8,true);
  }
  async webSocketClose(ws: WebSocket, _code: number, reason: string = "", _clean: boolean = false) {
    this.observe("in","close",enc.encode(reason));
    const a = this.state(ws);
    this.close(ws,1000, "closed");
    await this.releasePair(a);
    if (!a.key) return;
    if (a.kind === "pair" && a.role === "owner") {
      const meta = this.meta();
      if (meta?.pins[0] === a.key && meta.expires === a.deadline) this.closeAll("invitation owner disconnected");
    } else if (!this.sockets().some(other => other !== ws && this.state(other).key === a.key)) {
      for (const other of this.sockets()) if (other !== ws) this.send(other, { type: "left", key: a.key });
    }
  }
  webSocketError(ws: WebSocket) { this.close(ws,1011, "transport error"); }
  async alarm() {
    this.ensureSchema();
    const now = Date.now();
    this.ctx.storage.sql.exec("DELETE FROM pair_claims WHERE expires<=?",now);
    const claims=this.ctx.storage.sql.exec<{expires:number}>("SELECT MIN(expires) AS expires FROM pair_claims").toArray()[0]?.expires;
    const meta = this.meta();
    if (meta?.kind === "pair" && (meta.expires <= now || (meta.used && meta.cleanupAt <= now))) {
      this.closeAll("invitation closed"); await this.ctx.storage.deleteAll(); this.schema=false; return;
    }
    if (meta?.kind === "sync" && meta.expires <= now) {
      this.closeAll("sync room expired"); await this.ctx.storage.deleteAll(); this.schema=false; return;
    }
    const budget=this.ctx.storage.sql.exec<{window:number}>("SELECT window FROM login_budget WHERE id=1").toArray()[0];
    if (!meta && budget && budget.window+60_000<=now) {await this.ctx.storage.deleteAll();this.schema=false;return;}
    if (!meta && !budget && !claims) {this.closeAll("inactive room");await this.ctx.storage.deleteAll();this.schema=false;return;}
    let next = meta ? meta.kind === "pair" ? Math.min(meta.expires,meta.used ? meta.cleanupAt : Infinity) : meta.expires : budget ? budget.window+60_000 : Infinity;
    for (const ws of this.ctx.getWebSockets()) {
      const a = this.state(ws);
      if (!a.key && a.deadline <= now) this.close(ws,1008, "authentication expired");
      else if (!a.key) next = Math.min(next, a.deadline);
    }
    if (claims) next=Math.min(next,claims);
    if (Number.isFinite(next)) await this.ctx.storage.setAlarm(next);
  }
}
