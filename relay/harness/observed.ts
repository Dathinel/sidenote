// CI-only entry point. Production hooks are no-ops and expose no proof routes.
import worker, { Room as ProductionRoom } from "../src/index";
const marker = "SIDENOTE-PLAINTEXT-MUST-NEVER-REACH-RELAY";
const enc = new TextEncoder();
function capture(kind: string, bytes: Uint8Array, negative = false) {
  console.log(JSON.stringify({event:"relay-proof",kind,negative,bytes:Array.from(bytes,b=>b.toString(16).padStart(2,"0")).join("")}));
}
export default {
  async fetch(request: Request, env: Env & { RATE_ROOMS: Env["ROOMS"] }) {
    if (new URL(request.url).pathname === "/__proof/negative") {
      return env.ROOMS.getByName("proof:negative").fetch("https://proof.internal/plant-marker");
    }
    if (new URL(request.url).searchParams.has("__proof_rate")) {
      return worker.fetch(request,{...env,ROOMS:env.RATE_ROOMS});
    }
    return worker.fetch(request,env);
  },
} satisfies ExportedHandler<Env & { RATE_ROOMS: Env["ROOMS"] }>;
// Local emulation and proof logging must not turn the quota burst into several clock windows.
export class RateRoom extends ProductionRoom {
  protected override rateNow(): number { return 0; }
}
export class Room extends ProductionRoom {
  private negativeFrame = false;
  constructor(ctx: DurableObjectState, env: Env) {
    super(ctx,env);
    // CI observes pings through normal handlers; production retains hibernation auto-response.
    this.ctx.setWebSocketAutoResponse();
  }
  protected override observe(direction: "in" | "out", kind: string, bytes: Uint8Array) {
    capture(`${direction}:${kind}`,bytes,this.negativeFrame);
  }
  private audit(negative = false) {
    // Workerd's reserved tables are visible in sqlite_master but cannot be read by application SQL.
    const tables=this.ctx.storage.sql.exec<{name:string;sql:string}>("SELECT name,sql FROM sqlite_master WHERE type='table' AND name NOT GLOB '_cf_*' ORDER BY name").toArray();
    capture("storage",enc.encode(JSON.stringify(tables)),negative);
    for (const {name} of tables) {
      const rows=this.ctx.storage.sql.exec(`SELECT * FROM "${name.replaceAll('"','""')}"`).toArray();
      for(const row of rows) for(const value of Object.values(row)) {
        const bytes=value instanceof ArrayBuffer ? new Uint8Array(value) : ArrayBuffer.isView(value) ? new Uint8Array(value.buffer,value.byteOffset,value.byteLength) : enc.encode(typeof value === "string" ? value : JSON.stringify(value));
        capture("storage",bytes,negative);
      }
    }
  }
  override async fetch(request: Request): Promise<Response> {
    if (new URL(request.url).pathname === "/plant-marker") {
      this.ctx.storage.sql.exec("CREATE TABLE IF NOT EXISTS negative_control(value TEXT)");
      this.ctx.storage.sql.exec("INSERT INTO negative_control VALUES(?)",marker);
      this.audit(true);
      this.negativeFrame=true;
      try {this.observe("in","pair-binary",enc.encode(marker));}
      finally {this.negativeFrame=false;}
      this.ctx.storage.sql.exec("DELETE FROM negative_control");
      this.audit();
      return new Response("negative-control-written");
    }
    const response=await super.fetch(request);this.audit();return response;
  }
  override async webSocketMessage(ws: WebSocket, message: string | ArrayBuffer) {
    await super.webSocketMessage(ws,message);this.audit();
  }
  override async alarm() {await super.alarm();this.audit();}
}
