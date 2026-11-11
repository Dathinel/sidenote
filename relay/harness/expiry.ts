// Local test entry point only. Production configuration has no expiry route.
import worker, { Room as ProductionRoom } from "../src/index";

export default {
  async fetch(request: Request, env: Env): Promise<Response> {
    const match = /^\/__test\/expire\/([a-f0-9]{64})$/.exec(new URL(request.url).pathname);
    if (match && request.method === "POST") {
      return env.ROOMS.getByName(`pair:${match[1]}`).fetch("https://test.internal/expire", { method: "POST" });
    }
    const sync=/^\/__test\/sync-(pause|resume|status)\/([a-f0-9]{64})$/.exec(new URL(request.url).pathname);
    if(sync && request.method==="POST") return env.ROOMS.getByName(`sync:${sync[2]}`).fetch(`https://test.internal/sync-${sync[1]}`,{method:"POST"});
    return worker.fetch(request, env);
  },
} satisfies ExportedHandler<Env>;

export class Room extends ProductionRoom {
  override async webSocketMessage(ws:WebSocket,message:string|ArrayBuffer) {
    const exists=this.ctx.storage.sql.exec<{name:string}>("SELECT name FROM sqlite_master WHERE type='table' AND name='qualification'").toArray().length>0;
    if(exists && message instanceof ArrayBuffer && ws.deserializeAttachment()?.kind==="sync") {
      const state=this.ctx.storage.sql.exec<{paused:number}>("SELECT paused FROM qualification WHERE id=1").toArray()[0];
      if(state?.paused) {if(message.byteLength>1024) this.ctx.storage.sql.exec("UPDATE qualification SET observed=1 WHERE id=1");return;}
    }
    await super.webSocketMessage(ws,message);
  }
  override async fetch(request: Request): Promise<Response> {
    const action=/^\/sync-(pause|resume|status)$/.exec(new URL(request.url).pathname);
    if(action && request.method==="POST") {
      this.ctx.storage.sql.exec("CREATE TABLE IF NOT EXISTS qualification(id INTEGER PRIMARY KEY, paused INTEGER, observed INTEGER)");
      if(action[1]==="pause") this.ctx.storage.sql.exec("INSERT INTO qualification VALUES(1,1,0) ON CONFLICT(id) DO UPDATE SET paused=1,observed=0");
      if(action[1]==="resume") this.ctx.storage.sql.exec("UPDATE qualification SET paused=0 WHERE id=1");
      const rows=this.ctx.storage.sql.exec<{paused:number;observed:number}>("SELECT paused,observed FROM qualification WHERE id=1").toArray();
      return Response.json(rows[0] ?? {paused:0,observed:0});
    }
    if (new URL(request.url).pathname === "/expire" && request.method === "POST") {
      this.ctx.storage.sql.exec("UPDATE metadata SET value=json_set(value,'$.expires',?) WHERE id=1", Date.now() - 1);
      return new Response(null, { status: 204 });
    }
    return super.fetch(request);
  }
}
