# Linux MCP and operations

Server and Hub expose local MCP stdio and a stateless Streamable HTTP endpoint at
`http://127.0.0.1:6791/mcp`. `--mcp-port N` changes the port; `0` selects an ephemeral
port for qualification. The bind address is fixed to IPv4 loopback. The owner
removes `local/mcp/endpoint.json` and the private control socket on clean shutdown.
A second owner or an occupied port fails startup. Full-role behavior is unchanged.

Use the same absolute `--data-dir`, `--meetings-dir` and `--peers-dir` on service
and CLI commands. Keep state under a private service account. The local
`sidenote --mcp` client uses the existing private Unix socket on Linux Server/Hub;
it does not open another relay replica. Start the owner first. Local same-user
clients retain the existing tool contract, including proposals when enabled.

## Private network access

The owner chose private network access only. Forward an owner-managed SSH tunnel
or private Tailscale forwarding endpoint to **127.0.0.1:6791**. Merely installing
Tailscale does not forward loopback. Do not change the application bind address,
publish this endpoint, use Tailscale Funnel, or expose a plaintext HTTP port on a
network interface. Transport encryption and peer authentication belong to the
private forwarding arrangement. The application additionally requires a token on
every HTTP request. No host SSH/Tailscale settings are installed by this code.

The endpoint accepts numeric loopback Host values and rejects all browser Origin
headers. Configure a tunnel client with a URL such as
`http://127.0.0.1:16791/mcp`; it must preserve that Host value. Browser CORS clients
are intentionally unsupported. Use a native MCP client. The implementation uses
[rmcp's Streamable HTTP transport](https://modelcontextprotocol.io/specification/2025-11-25/basic/transports),
JSON responses and stateless requests, with a 64 KiB request-body limit, eight
request slots acquired before authentication and a five-second response deadline.
Accepted connections are capped at 32, headers expire after two seconds, and each
HTTP/1 connection closes after its response. It does not implement
an application TLS listener, OAuth enrollment, persistent HTTP sessions or SSE.

## Tokens and authorization

With the normal explicit path flags **before** the subcommand:

```sh
umask 077
sidenote --data-dir /private/state --mcp-access grant phone /private/export/phone-token
sidenote --data-dir /private/state --mcp-access revoke phone
```

The export directory must already be private, absolute and trusted. Grant creates
a new 0600 file and refuses an existing file or symlink. It prints only a fixed
success message. Transfer the file through a private channel into the client's
credential store, then remove the export copy. Never paste a token into a command,
log, issue, report or chat. The client supplies `Authorization: Bearer <token>`.
The server stores only a BLAKE3 digest in its private `local/mcp/clients.json`.
There are at most 128 client labels. Granting an existing label rotates its token;
use a new export filename. Revoke removes its digest. Protect host backups and
the client's credential store; revoke/rotate after suspected disclosure.

Every HTTP request, including initialization, requires current authorization.
The owner rechecks it before tool dispatch, and HTTP middleware rechecks before
returning the response. A completed response cannot be recalled. There is no
authenticated session to reuse after revoke; following requests return 401.
Loss, corruption, unsafe permissions or a stale digest fails closed. Same-user
account compromise and privileged host administrators remain outside this boundary.

Network clients can read meetings, tasks, teams, projects, memory and existing
daily reports when their product features are enabled. They cannot propose
memory, pair devices, configure the service, edit tasks or resolve conflicts.
This is device-wide read access, not a per-meeting access-control policy.
Feature switches are not content security boundaries. Phone interoperability and
the production tunnel configuration need separate qualification.

## Operator checks and logs

```sh
sidenote --data-dir /private/state --ops status
sidenote --data-dir /private/state --ops logs
```

Status exits 0 when ready, 1 when degraded and 2 when the owner is unavailable.
`live` means the private owner endpoint answered. Readiness requires an observation
within five seconds, no reconciliation errors, a connected relay when enabled,
and no failed, blocked or unreadable queue entries. Queued work is reported but
does not by itself fail readiness. Directory scans stop after 4,096 entries and
mark incomplete/unreadable results. Counts can change during a concurrent scan.
Router failure artifacts can remain until a subsequent attempt replaces them.

Peer health reports the configured count, fixed transport state and the last
time the owner observed a connected transport during this process lifetime.
This is **not** a remote data acknowledgment or last successful peer sync time.
No connected observation is reported after a restart until one occurs.

Exported status omits identities, locators, paths, meeting/task IDs and content.
Operations logs contain only timestamps and fixed event names. Linux Server/Hub
listener and notes logging use this bounded 128-event private file instead of
writing arbitrary worker messages and IDs. Existing logs from earlier versions
are not rewritten or exported. A corrupt event file produces a fixed error;
it is not echoed. Service stderr uses fixed categories and status values.

## Resource qualification

Run `tools/prove_linux_step4.py <linux-binary> hub` and `server` only on the isolated
Linux box with the step-2 credential provider. It generates synthetic meetings and
disposable tokens under private temporary directories. It verifies transport,
denial/revocation, queues, logs, restart and resource assertions. It does not
execute models, use a real phone, touch installed Windows data or prove real
Syncthing behavior. Run the server build with no desktop features.

The proof measures ten-second idle windows after startup, sixty completed HTTP
tool requests/disconnect and restart. RSS and CPU include all observed descendant
processes; CPU is a percentage of one core. Budgets are 96 MiB peak idle RSS,
2% of one core idle CPU, and at most 16 MiB retained RSS growth after work.
Display/GPU/ONNX runtime mappings and GPU device descriptors fail qualification.
`STEP4_BASELINE=1` records an initial baseline without enforcing budgets; it is
never an acceptance run. Measurements are for the bounded synthetic, model-free
corpus, not a production capacity estimate or inference budget. Raw observations
and exact accepted heads belong in the external linux-step4.md proof.
