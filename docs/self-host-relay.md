# Run your own Sidenote relay

The relay is a small Cloudflare Worker, an HTTP/WebSocket service that Cloudflare
runs for you. Its Durable Object coordinates the devices in each room. You do not
need a server, a domain, or a Cloudflare account to try the local version below.

This guide is for the opt-in **Relay sync (preview)**. It is not a production
release sign-off. The local steps were executed on Windows with synthetic data.
No Cloudflare login, account API call or deployment was performed.

## Before you start

Get a Sidenote source checkout and open a terminal in its `relay` directory.
Install Node.js 22 or newer and Bun using their official installers. This check
used Node 24.18.0, Bun 1.4.4 and the lockfile's Wrangler 4.158.0. Wrangler is the
Cloudflare development tool; use the pinned copy installed with this project.
The commands below use Windows PowerShell. On another OS the curl executable is
usually named `curl`; that platform has not been qualified here.

Run these commands in order. They are local and require no Cloudflare account:

```powershell
node --version
bun --version
bun install --frozen-lockfile
bun run typecheck
bun test tests
node node_modules/wrangler/bin/wrangler.js --version
```

Installation downloads development dependencies. It does not create a relay on
Cloudflare. Do not use a globally installed Wrangler with a different version.

## Configuration

The tracked `relay/wrangler.toml` already contains the local working values:

| Value | Meaning and what to change |
| --- | --- |
| `name = "sidenote-relay"` | Your Worker's name. Choose your own unique name before publishing. |
| `main = "src/index.ts"` | Worker entry point. Keep this. |
| `compatibility_date = "2026-11-09"` | Pinned Cloudflare runtime behavior, separate from Sidenote's data schema. |
| `workers_dev = true` | Allows a workers.dev address after an account owner publishes. |
| `send_metrics = false` | Disables Wrangler usage metrics for this project. |
| `[observability] enabled = false` | Keeps Worker request logging off by default. Do not add message-body logging. |
| Durable Object binding `ROOMS`, class `Room` | These names must match the code. Keep both. |
| Migration tag `v1`, `new_sqlite_classes = ["Room"]` | Creates SQLite-backed room state. Preserve migration history on upgrades. |

No API token, group key, pairing code or device identity belongs in this file.
No KV, R2, D1 or queue binding is needed. Keep Sidenote's local replica databases
outside Syncthing; do not copy a device identity to another installation.

## Start the local Worker

From the same `relay` directory, run:

```powershell
node node_modules/wrangler/bin/wrangler.js dev --local --ip 127.0.0.1 --port 8787 --inspector-ip 127.0.0.1 --inspector-port 0 --show-interactive-dev-session false --log-level error --persist-to ../artifacts/relay4-local/state
```

Leave this process running. It binds numeric loopback, so only this PC can reach
it. It creates local room metadata under `artifacts/relay4-local/state`; this is
not Cloudflare storage. Use Ctrl+C when you finish. Stop this process before
running the automated local-check helper, because both use port 8787.

In a second terminal, check it:

```powershell
curl.exe --fail --silent --show-error --noproxy 127.0.0.1 http://127.0.0.1:8787/health
curl.exe --fail --silent --show-error --noproxy 127.0.0.1 --head http://127.0.0.1:8787/health
```

GET returns `sidenote-relay-v1`. HEAD returns HTTP 200 and no body. Both disable
caching and need no credentials. A health response proves this endpoint is
reachable; it does not prove pairing, WebSocket delivery or device convergence.

Every command in the blocks above was executed locally. Captured output and the
exact command list are in `artifacts/relay4-local/commands.json` and adjacent logs.
The tracked `tools/check-local-relay.py` repeats this sequence from the checkout
root, launches its own hidden local Worker and stops only that process tree.
It refuses to reuse a service already listening on port 8787.

## Point Sidenote at it

1. Use synthetic data in two isolated installations for a local test. Enable
   Settings > This PC > Sync > Labs > **Relay sync (preview)** on both.
2. Open **Advanced**, enter `ws://127.0.0.1:8787` in **Relay address**, and press
   Enter. Do not append `/health`, `/sync`, a room name, credentials or a query.
3. Choose **Show a pairing code** on one installation. On the other choose
   **Have a code from another device? Enter it**. Enter the eight digits there.
   Never paste the code into logs or an issue.
4. Compare both device names and the safety phrase. Confirm on both devices.
   Codes expire after ten minutes and can be used only once.
5. Keep both installations online together and make a synthetic task edit.
   Check the edit on the other installation. The local automated app test also
   checks offline field merges, migration, rollback and incompatible versions.

**Last relay contact** in Advanced is this PC's last successful health check,
displayed in local time. It survives restarting Sidenote and is retained during
an outage. It is not a delivery receipt. A bad address and an unreachable relay
have separate messages. Local edits remain on this PC during an outage.

An authenticated newer schema/capability requirement makes an older client
read-only for relay data. It says: "Another device runs a newer Sidenote. Update
this one to keep syncing." Folder-only data still works. The requirement is
durable and cannot be lowered by restarting or downgrading. Update every device
before using the preview. Pre-negotiation clients cannot exchange documents with
this build. An offline device cannot learn an upgrade it has not encountered yet.

## A real Cloudflare account: not executed here

Local loopback cannot connect two physical PCs. For that, the account owner must
create a Cloudflare account, choose its Workers plan and workers.dev subdomain,
authorize the pinned Wrangler, and publish the configured Worker. Follow the
official [first deployment guide](https://developers.cloudflare.com/workers/get-started/guide/)
and [Wrangler command reference](https://developers.cloudflare.com/workers/wrangler/commands/workers/).
Those account and publication actions were not run, and no runnable login or
publication command is included in this locally tested command list.

After publication, set both apps to the returned **WSS origin on port 443**,
for example `wss://your-worker.your-subdomain.workers.dev`. Use your real address,
without a path. Pair again and perform the same synthetic end-to-end check.
Real-account provisioning, TLS/routing, billing and two-device operation still
need separate release evidence. Do not infer them from the local emulator.

## Revoke, rotate, upgrade and recover

**Removing a device is not available in this preview.** The previous local-only
rotation did not revoke other peers' old-group sessions. Remove is hidden and
the backend refuses it. Do not use this preview with a lost or stolen paired
device. A prior removal must not be treated as proof that access was revoked.
Distributed retirement, offline notification and old-history isolation still
need implementation and independent re-review before removal can return.

A device refuses a group it has durably retired, in either pairing direction.
After switching groups, earlier local groups cannot be adopted again. This is a
local guard, not a distributed revocation system. Never display, export or
manually edit group keys or delete retirement records to bypass that guard.

Update Sidenote on all devices together; do not delete capability records to
make an older build writable. If a newer requirement makes the relay read-only,
update this device. Neither changing the relay URL nor its abuse allowlist
rotates encryption keys or removes a member from an existing group.

For a Worker update, keep the binding/class names and migration history, run the
local commands again, then have the account owner perform a separately approved
publication. A changed address is saved per device in Advanced. Changing hosting
does not replace lost device keys. Lost-device recovery and independent security
review remain release work.

Each device stores a full replica. The relay has **no offline content backlog**:
both devices must be online at overlapping times to exchange changes. An outage
does not delete local edits. Back up device data with Sidenote stopped, including
the local replica and its OS-protected identity, in a private backup outside
synchronized folders. A backup restored to another Windows account cannot unwrap the DPAPI-protected keys.
Do not treat relay room state as a backup. Turning preview off returns to the folder collection and retains the relay collection. If newer folder files arrived while preview was on, disabling safely refuses and asks you to keep those newer files. Do not restore older backups over them. Re-enabling applies folder-field changes against the previous verified snapshot. Older previews without that snapshot safely refuse an ambiguous changed import.

## Costs and limits

As checked on 11 November 2026, Cloudflare offers SQLite-backed Durable Objects
on Workers Free. Free has daily quotas; exceeding them can stop service. Workers
Paid has a minimum $5 monthly charge plus usage. Worker requests/CPU and Durable
Object requests, active duration and stored metadata are separate meters. More
devices, reconnects or hostile public traffic can increase cost. This is not an
estimate or a promise that your relay will be free. Check current
[Workers pricing](https://developers.cloudflare.com/workers/platform/pricing/)
and [Durable Objects pricing](https://developers.cloudflare.com/durable-objects/platform/pricing/)
before choosing a plan. Monitor usage in the dashboard and set appropriate CPU
limits; a CPU limit is not a cap on every billing meter.

The relay limits a frame to 128 KiB, a room to 32 sockets and a pairing exchange
to 64 messages. A group supports at most 16 enrolled devices. Authentication, invitation and rate limits can close connections.
Room routing/membership metadata expires; no task-content queue is retained.
The app currently bounds a document to 32 MiB and its replica store to 2 GiB.
These are safety bounds, not capacity promises. Larger collections need release
qualification. Cloudflare can observe IPs, public keys, room identifiers, timing
and sizes. Encrypted content and group keys stay with the devices. A paired
device can read and change shared content, so approve only devices you trust.

## Optional abuse controls

`[vars] ALLOWED_GENESIS` in `relay/wrangler.toml` is an optional comma-separated
list of lowercase 64-hex **public** device keys. Empty is the default and keeps
open enrollment. When set, a sync room's genesis key must be listed, and every
device that shows a pairing code must be listed. Guests still need valid
pairing and enrollment proofs. Include the trusted pairing hosts, not private
keys or pairing codes. Changing this setting is not device revocation.

Test the configuration only with synthetic identities and numeric-loopback
Wrangler using the existing local commands above. Tests cover empty/default,
listed and unlisted keys. No account configuration was performed in this round.

The Worker caps pending pairing rooms at eight per source IP and three per
verified host key across rooms. Claims expire after ten minutes and release on
completion or owner disconnect. Source IP comes from Cloudflare's trusted
`CF-Connecting-IP` header; local tests use synthetic header values. Each socket
has a 4 MiB/s input budget, a 600-message/s cap and the existing frame limits.
These bounds do not make an open public endpoint immune to a distributed attack.

Before hosting, the account owner should add a Cloudflare rate-limiting rule for
`/pair/` and `/sync/` upgrade requests, keyed by source IP, and monitor requests,
Durable Object activity and billing. Choose thresholds for the actual fleet and
plan. Edge-rule setup and effectiveness require the real account and were not
run locally. See the official [rate-limiting guide](https://developers.cloudflare.com/waf/rate-limiting-rules/).

At enable, Sidenote prepends `/local` to the share's `.stignore` ahead of existing
rules, preserving those rules. It leaves that exclusion in place when disabled,
because the retained replica and protected keys must stay on this PC. The
replica content is plaintext at rest; DPAPI protects its keys, not all content.
