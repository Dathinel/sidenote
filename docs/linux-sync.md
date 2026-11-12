# Continuous server sync

Server and Hub own a relay session and a one-second ingestion loop without a window.
The service holds `local/service.lock`; an enabled replica also holds SQLite's
exclusive writer lock. Start the service with the step-2 unit and credential setup
in [linux-server.md](linux-server.md). Stop it before restoring a matched replica
and credential backup. A second owner exits instead of taking over its database.

## File transport and ingestion

Syncthing remains an independent, owner-managed transport. Sidenote does not start,
stop, adopt or configure that daemon. If it is already a systemd user service, the
owner can add `Wants=` and `After=` for that actual unit in a Sidenote drop-in.
Ingestion also starts without it and retries arriving files each second, so a
transport restart does not require a Sidenote restart. Do not point two Syncthing
instances at the same configuration directory.

Preserve existing share boundaries: device identity/configuration and all `local/`
data, SQLite/WAL/SHM, locks, control sockets and credentials stay local. Relay setup
adds `/local` to `.stignore`; retain the existing device exclusions too. Never share
a replica database. Enabled relay domains are authoritative through the existing
relay-mode migration/locks. File journals continue to serve the other supported
stores; the ingestion loop does not copy relay projections back into journals.

The loop validates/reconciles transported journals and scans complete local/peer
meetings. `local/sync/index.json` records all source identities and an
opaque revision, not a second copy of meeting text. Incomplete or invalid inputs
stay in place for retry and raise an error count. Unchanged inputs do not advance
the ingestion generation. Index state survives restart; source meetings/journals
are never rewritten by the index. Remote meetings are discovered, not automatically
queued for model/notes processing in this step.

Readiness follows the existing core's saved-summary/notes rules. A valid saved
summary can be ready while its manifest still says pending. Truncated/unreadable
summaries are retried. The per-meeting content fingerprint includes title, summary,
notes, transcript and routed revision, so a later edit updates the existing entry.

## Retained source revisions

Every source is retained as `(device, id, revision)`. Identical revisions coalesce
into one logical meeting count while inventory preserves every source. Divergent
revisions have `conflict=true` and increase the logical conflict count. Inspect all
source revisions and correct the authoritative source explicitly; ingestion never
chooses a winner or overwrites peer files. Task conflict resolution is unchanged.

A failed, missing or unreadable source has `pending=true` and keeps its last-good
revision across restart. Pending changes do not advance the content generation.
Recovery to the same content is not a second ingestion. This index has no meeting
deletion protocol: absence never silently deletes prior successful state.

## Owner commands

The control pipe is private and can carry document text or pairing material.
Do not tee it into logs. `status` contains counts, a public identity fingerprint
and connection state; it contains no invitation, group locator or document body.
Pairing material appears only in the explicit `pair-state` response. Pass join
codes through stdin, never flags, environment variables or shell history.

| Request | Behavior |
| --- | --- |
| `{"op":"status"}` | Ingestion generation, source revision, meeting/source/pending/conflict counts, error/task/peer counts, relay state |
| `{"op":"configure","enabled":true,"address":"wss://relay.example.test"}` | Enable relay with the existing backup/migration procedure; address is an origin |
| `{"op":"host"}` / `{"op":"join","code":"<private invitation>"}` | Begin existing PAKE pairing; poll `pair-state` |
| `{"op":"pair-state"}` | Private invitation or SAS/attempt while pending; terminal pairing phase |
| `{"op":"confirm","attempt":1,"phrase":"<matching local SAS>","accept":true}` | Explicit approval; wrong phrase/stale attempt is rejected; false declines |
| `{"op":"cancel"}` | Cancel pending pairing and erase its retained material |
| `{"op":"list"}` / `{"op":"add","task":{"id":"stable-id","task":"..."}}` | Read/add manual tasks |
| `{"op":"edit","id":"stable-id","fields":{"owner":"..."}}` / `{"op":"delete","id":"stable-id"}` | Edit/delete with the existing conflict-preserving versions |
| `{"op":"conflicts"}` | Current conflicting alternatives |
| `{"op":"resolve","conflict":{},"chosen":{}}` | Supply a current conflict and one of its returned choices; stale choices are checked by the existing resolver |
| `{"op":"offline"}` / `{"op":"online"}` | Stop/restart the session; local edits remain durable |
| `{"op":"check","meeting":"100-2","task":"...","checked":true}` / `{"op":"checks","meeting":"100-2"}` | Existing checked-state API, through the active transport's ownership rules |

Both devices must compare the displayed SAS and explicitly approve. An invitation
alone does not approve a device. Treat `unconfirmed` as uncertain delivery and
check the other device before recovery. This command does not automatically accept
pairing or pick conflict winners. A transport outage uses the existing reconnect
backoff. Restart uses the same encrypted identity and replica. A stopped owner
releases its session and workers; systemd retains responsibility for restart.

## Source inventory and bounded reads

`status` is a compact summary. `inventory`, `list`, `conflicts` and `choices` return
`{revision, items, next}` pages (at most 32 items and 16 KiB of item data).
Pass `next` back as `cursor`. `SYNC_CONTROL_CURSOR_STALE` means restart that read;
`SYNC_CONTROL_ITEM_TOO_LARGE` is a structured error for an unrepresentable item.
The one-MiB reply cap is unchanged and no longer closes a healthy connection.
Status and inventory read the last published snapshot while ingestion is busy.
Conflict pages contain small revision-guarded selectors. Pass a selector as
`conflict` to `choices` to page its alternatives, or to `resolve-choice` with a
zero-based `choice` index to resolve it. This avoids echoing a large conflict in a
request. Even conflicts with many alternatives remain inspectable/resolvable.


## Private local transport

Use the owner's local control connection for headless edits and pairing:

```sh
sidenote --sync-control --data-dir /absolute/private/sidenote
```

Input and output are newline-delimited JSON, one response per request. Commands
run in the owner; this client does not initialize a key or open SQLite. The socket
is `local/sync/control.sock`, mode 0600 in a 0700 directory. Both ends check the
connecting UID. Frames are bounded to 64 KiB in and 1 MiB out, with two-second
socket deadlines. A timed-out mutation can have committed: inspect its result
before retrying, and supply a stable task ID when adding a task.


## Qualification and remaining proof

`tools/prove_linux_step3.py <linux-binary> --role server` and `--role hub` use a
loopback routing fixture and a Full-role peer built as `examples/sync_peer`. The
peer calls the same owner/pairing/store APIs. Its path is confined to the named
OS temporary proof directory; it has no Velopack, UI or installed-listener startup.
The fixture requires Node's `ws` package. It verifies transport authentication and
forwards opaque frames; it does not qualify the Cloudflare relay's policy, limits
or persistence. Tests use two provisioned synthetic credential directories.

For an isolated fixture dependency, use `npm install --ignore-scripts --no-audit
--no-fund --package-lock=false --prefix /private/proof/node ws@8.18.3` and set
`NODE_PATH=/private/proof/node/node_modules` for the proof process. This dependency
belongs to the qualification runner, not the server executable.


## Proof scope

`--peer-command-json` takes a command adapter with `{root}` and `{relay}`
placeholders. A future Windows adapter must choose native isolated proof paths,
forward this same private NDJSON protocol, provide loopback relay access and stop
its peer on EOF. Pairing/offline/conflict/reconnect/restart assertions remain the
same. The Linux-only raw-journal arrival fixture is separate from those convergence
assertions. Actual Windows Full pairing remains unverified until that peer is
authorized. This run also does not establish Syncthing-daemon, physical reboot,
phone, network-MCP or inference qualification.
