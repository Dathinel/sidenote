// Local test entry point only. Production configuration has no expiry route.
import worker, { Room as ProductionRoom } from "../src/index";

export default {
  async fetch(request: Request, env: Env): Promise<Response> {
    const match = /^\/__test\/expire\/([a-f0-9]{64})$/.exec(new URL(request.url).pathname);
    if (match && request.method === "POST") {
      return env.ROOMS.getByName(`pair:${match[1]}`).fetch("https://test.internal/expire", { method: "POST" });
    }
    return worker.fetch(request, env);
  },
} satisfies ExportedHandler<Env>;

export class Room extends ProductionRoom {
  override async fetch(request: Request): Promise<Response> {
    if (new URL(request.url).pathname === "/expire" && request.method === "POST") {
      this.ctx.storage.sql.exec("UPDATE metadata SET value=json_set(value,'$.expires',?) WHERE id=1", Date.now() - 1);
      return new Response(null, { status: 204 });
    }
    return super.fetch(request);
  }
}
