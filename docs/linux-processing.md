# Unattended Linux processing

Use the Server role for routing. Hub owns sync/MCP
and starts none of these workers. No Hex installation or display is required.
The listener connects only when the configured meetings root's parent contains
Hex discovery metadata; recording, capture, vocabulary and inference are unavailable
without their separately qualified dependencies. Missing helpers leave durable
notes state and do not stop sync/MCP. Scripts engine selection is unsupported on
Linux and degrades processing. Agent-backed notes and daily checks are currently
blocked on Linux: `agent runners need network; not yet supported on Linux`.

Configure external file synchronization to deliver complete meeting folders into
the Server's configured meetings directory. Its manifest and original transcript
must pass the recording eligibility guard using one pinned snapshot. Symlinks,
nonregular files, multiply linked inputs and oversized inputs are rejected.
Discovery does not open inputs before the queue captures them. Peer inventory directories
are read-only projections; this release never copies or edits them for processing.
Only one designated processing server should receive a given input for processing.
Queue locks deduplicate within that server; there is no distributed work lease.
The separate Windows PowerShell pipeline is owned on pipeline/4-errors. Linux
uses native Rust queues and their transcript/manifest contract, without executing
or modifying those scripts. A future portable script adapter needs its own lane.

Routing and notes run as separate bounded service threads sharing the replica.
External runners/inference use the existing contained child process frontend.
Successful routing revisions prevent periodic duplicate work. Failed attempts
preserve earlier output and remain pending with a 30-second automatic retry delay.
Changed input becomes a new request. Pending generations survive crashes. Notes recover Running jobs,
preserve existing summaries and publish through the settlement transaction store.
Service stop cancels runner waits, stops queue loops and joins workers before
disconnecting the shared replica. SIGKILL is recovered on the next start.

## Daily schedule

Linux currently retains an incomplete attempt rather than launching an agent or
publishing a report. Keep this schedule disabled until agent runners are qualified.
Local-file Git/tar work remains offline; network-backed KB fetches are unavailable.

Write private `service-schedule.json` under the data directory:

```json
{"enabled":true,"hour":8,"minute":30,"runner":"Daily check (Opus)"}
```

The host's local timezone defines the day/time. Missing configuration is disabled.
Also enable the existing device and feature daily-check consent flags and configure
a supported read-only runner. The first service tick after the scheduled time
catches up today's report. Completed reports are immutable for automatic runs;
failed/interrupted attempts retry after 15 minutes. Corrupt schedule/receipt state
fails closed. There is no backfill of missed earlier days. There is no timer
launching a second replica owner.

## Optional CPU model files

`sidenote --cpu-models list` prints catalog identifiers, sizes and licenses.
`sidenote --cpu-models fetch ID` explicitly downloads only a pinned non-Hex model
from the existing HTTPS catalog. It uses no environment proxy, checks allowed
redirect hosts (including the provider-documented US GCP edge), exact size and SHA-256, resumes partial downloads and publishes the
installation receipt last. `--cpu-models verify ID` re-hashes every installed file.
Both use the selected data directory; no fetch runs at startup. Hub rejects them.
Storage must pass the private-directory check; partial files inherit umask 0077.

The exact GCP host is documented by [Hugging Face](https://huggingface.co/docs/hub/models-downloading#downloading-behind-a-proxy-or-firewall); HTTP, arbitrary subdomains, userinfo and alternate ports remain denied.

Files are not inference capability. The Linux ONNX loader still refuses the
unqualified runtime. Validate the CPU libonnxruntime build/version, dependency
licenses and model performance separately before enabling inference. Existing
catalog license labels are metadata, not a new license review.

## System service, independent of login

The `sidenote-server@.service` system template uses a dedicated non-root numeric uid,
private `/var/lib/sidenote-UID` state and systemd encrypted credentials. It explicitly
requires `user@UID.service`, then launches `sidenote-server-worker.service` in that
user manager. Only the trusted system-unit launcher contacts that manager. The system
unit waits for the worker, propagates failure/restart and explicitly stops it. This
requires neither interactive login nor linger. Its install target is `multi-user.target`.
The template and Linux service binary both reject uid 0. The actual user worker
uses a private user namespace, a read-only filesystem and private temporary/device
namespaces. Only `/var/lib/sidenote-UID` (state, meetings, peers and models) is
writable through `/state` (including models), `/meetings` and `/peers` inside a
`RootDirectory` rooted at that private directory. This keeps ancestor ownership
validation intact when host root is unmapped in the user namespace. System binaries,
libraries, TLS/DNS configuration and account metadata are bound read-only.
`ProtectHome=tmpfs` hides home directories and other runtime directories;
the manager bus is absent from the worker root and its address is removed from the
worker environment. Helpers launch directly inside the same sandbox and cgroup.
A per-helper empty network namespace blocks host TCP and abstract Unix sockets;
no interfaces are enabled. Model downloads use Rust HTTP in the parent, which
retains networking. Claude/Codex/OpenCode runners fail closed before launch.
A per-helper PID namespace kills all descendants when its init exits, including
children which call setsid. Parent-death signals close launcher/owner death paths.
The entire worker tree is limited to 512 MiB, 32 tasks and one CPU; helper startup
refuses an owner cgroup without those aggregate limits. The parent's network families are limited
to Unix, IPv4 and IPv6; mount syscalls, privilege gains, realtime scheduling and
personality changes are denied. No helper has a user-manager client or broker.
Unprivileged user namespaces must be enabled; namespace setup failure stops startup.
Credential and state semantics follow [systemd.exec](https://www.freedesktop.org/software/systemd/man/latest/systemd.exec.html).

Create an account reserved for Sidenote. Build the server and Linux worker with the
locked graph. As root, run `packaging/install-linux-server.sh UID BUILD_DIRECTORY`.
Provision `sidenote-key` using the step-2 encrypted-credential procedure into
`/etc/credstore.encrypted/sidenote-UID`, mode 0600, and retain its protected backup.
Never print or put key bytes in argv. No desktop keyring or plaintext fallback.
Then `systemctl enable --now sidenote-server@UID.service`.

The first launch creates private state; set consent and schedule via the service
account's data directory. Synchronize synthetic inputs before production use.
MCP remains 127.0.0.1 only; use the step-4 token lifecycle and private tunnel guide.
The unit grants no public bind option. Journal output uses redacted service events;
configure host journald retention separately. Memory/CPU/task ceilings are initial
operational limits and may require explicit qualification for larger workloads.

Before upgrade: stop this unit, retain prior binaries and a protected state backup,
replace both binaries, start and inspect `--ops status`. Roll back by stopping,
restoring the prior binaries/state as needed, then starting. Never start an older
binary against a newer incompatible schema. Disable with `systemctl disable --now
sidenote-server@UID.service`; retain state/credential for recovery. The older
`sidenote.service` remains a user unit and needs explicit lingering to start at boot.
Do not run both units against one state directory.

Cold system-manager start and forced restart can be qualified in a box. A physical
reboot/power-loss claim requires actually controlling and observing that reboot.

## Annotation capability gate

Immutable image pinning is unavailable on Linux: the current runner accepts paths,
while Unix advisory locks cannot enforce the Windows share-mode guarantee against
replacement or writes. `immutable_image_pinning_supported()` is false there. Six tests
whose success requires that guarantee retain their assertions but are explicitly
ignored on non-Windows targets with that reason. The Linux capability regression
still executes and asserts Unsupported before sending. Parsing, redaction, approval,
ordering, schema and other portable annotation tests continue to run. Production
pinning continues to fail closed; this gate does not enable Linux image sending.
