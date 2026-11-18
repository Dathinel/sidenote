# Service roles and Linux build

Build the service without desktop dependencies:

```sh
cargo build --locked --target x86_64-unknown-linux-gnu --no-default-features --features server
cargo clippy --locked --target x86_64-unknown-linux-gnu --no-default-features --features server -- -D warnings
```

`desktop` is on by default. It owns Rune and wgpu. `ui-kit` requires `desktop`.
The Linux desktop is not qualified in this lane. Its launch returns
`UNAVAILABLE_ON_PLATFORM`. Windows retains its desktop and ASR builds. The checked-in
Cargo target remains Windows GNU; select `--target x86_64-unknown-linux-gnu` when building on Linux.
Windows build and packaging scripts select `stable-x86_64-pc-windows-gnu` and
`--target x86_64-pc-windows-gnu` explicitly; the root toolchain retains the Windows GNU pin. Set `RUSTUP_TOOLCHAIN=stable` on Linux.

Start an isolated service with one of the persisted device roles:

```sh
./target/x86_64-unknown-linux-gnu/debug/sidenote --server --device-role server --data-dir /absolute/private/data
```

`--device-role full|server|hub` updates the device-local `device.json` while preserving
the device identity. Existing configurations default to `full`. Restart after changing
the role. Ordinary launches of `server` and `hub` run the service without a window.
On Windows, Full `--listen` and `--server` use the dev.19 listener without opening
its desktop. Server and Hub use the supervisor; its relay/journal duties skip Full.
Legacy config reads remain unchanged. They briefly hold `local/config.lock`, and
`Config::overlay_profile` can still open a relay Preview retained by the process-wide
registry in `relay_preview::get`. That inherited F1 writer-lock path remains unresolved
by decision; these fixes remove only the supervisor's additional ownership.
A missing device.json uses Full without logging or creating device identity.
A locked, unreadable or invalid device file logs one fixed fallback message and
runs as Full. Explicit one-shot commands remain available.
The local configuration and `local/` must stay excluded from file synchronization.

| Role | Service | Desktop | Processing |
| --- | --- | --- | --- |
| Full | Windows dev.19 listener; no relay/journal duties | Windows desktop | Configured backends |
| Server | Ordinary launch or `--server` | Never | Configured backends |
| Hub | Ordinary launch or `--server` | Never | Disabled |

The Full listener checks `service::ui_allowed()` before starting the icon monitor.
It requires Full, a desktop build and a desktop session. Server and Hub never start
the icon. The icon module is compiled only on Windows with `desktop`; a server-only
build excludes it entirely.

The Server and Hub supervisor holds a device-local process lock, reconciles transported
journals every 30 seconds and owns an enabled relay session. Windows also runs the
existing listener, completion watcher and processing queues. Missing Hex leaves Hex features unavailable;
it does not stop reconciliation. Hub skips Hex and processing. Status uses fixed labels
without paths, meeting identifiers, content or transport errors. SIGTERM on Unix and
Ctrl+C stop workers, disconnect relay and release locks. Windows child jobs are closed
and child processes are reaped on service shutdown. For every Windows role, an
independent 250 ms stop-marker watcher exits the listener before the updater's
8 second wait, without waiting for catch-up or worker joins. Velopack callbacks run
for every Windows role, including builds without desktop features.

MCP keeps the existing local stdio transport. Launch the same binary with `--mcp` from
an MCP client. The service status advertises this transport, not a network listener.
Authenticated phone MCP and continuous transport/pairing work remain later audit steps.

On Linux, data defaults to `$XDG_DATA_HOME/sidenote` or `~/.local/share/sidenote`.
XDG configuration and state roots have separate helpers. Relative XDG roots are ignored.
On Linux an explicit data directory also isolates default Hex and peer paths.
Windows keeps the dev.19 Hex/captures defaults; set explicit meeting/capture overrides
for isolated Windows work. Explicit meeting and peer directory overrides take precedence.
Linux writes use private files, pinned directory descriptors, no-follow opens, atomic replacement and nonblocking `flock`.

Linux supports encrypted relay identities, bounded sync/notes/routing subprocesses and
native CLI executable discovery. Legacy PowerShell scripts, updates and desktop operations
return `UNAVAILABLE_ON_PLATFORM`. Syncthing remains an external transport.

## Linux step 2: identity and workers

Server and Hub provision/reload the local relay identity before starting background work.
The public device id remains nonsecret `device.json` metadata; signing and group keys use
ChaCha20-Poly1305 envelopes in `local/relay/replica.sqlite`. The database and lock files are
0600 and their directory is 0700. Windows still uses DPAPI and Windows jobs.

The master encryption key uses an explicitly supplied `CREDENTIALS_DIRECTORY/sidenote-key`
first. It must contain exactly 32 bytes, owned by the service uid, with no group/other
permissions; its directory must also be private. systemd provides 0400/0500 runtime
credentials, stricter than 0600/0700. Symlink credential files/directories are rejected.
An invalid credential fails closed without trying another provider. No key is logged.

Otherwise a session bus and unlocked Secret Service keyring are required. Install
`libsecret-1-0` and a Secret Service provider (for example GNOME Keyring). Sidenote calls
libsecret's synchronous lookup/store API using the schema `org.sidenote.Identity`,
attributes `application=sidenote` and a stable `install` attribute, storing a random master key in the default persistent
collection on first use. Cargo.lock contains no libsecret/keyring client crate, so the
small FFI adapter loads libsecret at runtime. A private provider lock serializes initial
creation across processes. No session keyring means an explicit startup failure; no
plaintext or newly generated substitute key is used to decrypt an existing identity.
These calls can prompt the owner to unlock the desktop keyring. For an unattended host,
use the credential provisioning below instead.

Notes, routing and native CLI helpers require a systemd user service with delegated
memory, pids and CPU controllers. Otherwise helper construction returns the fixed
`UNAVAILABLE_ON_THIS_LINUX_SETUP` error; no uncontained helper is executed.
Each worker runs in a transient Type=exec service under a fresh 512 MiB, 32-task,
one-CPU slice. Nested helper slices are children of the owning worker's slice, so
the outer budget accounts for the entire tree. A guardian in the service watches
pidfds for both the owning process and its launcher, checks PID start times, and
exits when either disappears or when the real worker exits. KillMode=control-group
and KillSignal=SIGKILL then kill all forked descendants, even after setsid.
The 120-second wall deadline kills the complete service; RuntimeMaxSec supplies an
independent manager-enforced deadline. This replaces an aggregate CPU-seconds
counter: every tree gets at most 120 seconds of wall time at one CPU. The existing
per-process 512 MiB address-space/120 CPU-second rlimits, core suppression,
no-new-privileges and PDEATHSIG remain additional protection. The main service is
a child subreaper; it kills/reaps adopted proxies and remaining orphans at shutdown.
The main unit's 1 GiB cap applies to that unit; separate worker slices have their
own aggregate caps. The sync-only worker retains its narrower non-forking path.

SQLite state ancestors must be root/current-user owned and not group/other
writable. A root-owned sticky directory is accepted only when its immediate next
hop is a non-writable user-owned directory (for example /tmp/private-user-dir).
Symlink ancestors are rejected. The final store directory must already be private;
Sidenote does not chmod an unsafe path into apparent safety. SQLite uses NOFOLLOW,
and the backend fstats the actual open database/WAL/SHM descriptors through procfs,
requiring owned regular 0600 files. Missing procfs verification fails closed.

Secret Service uses an additional stable nonsecret install attribute, SHA-256 of
the validated canonical provider-directory path. Its namespace matches that
directory's provider.lock. Different XDG roots cannot replace each other's keys;
an existing item is only read, never rewritten. Creation is confirmed by a second
lookup while the lock is held. This prerelease namespace change does not migrate
old app-only keyring items: restore/re-enroll test state deliberately rather than
silently substituting a key. Lock acquisition and lookup/store share a five-second
initialization deadline. A live cancellation thread drives GCancellable and SIGTERM
cancels initialization. Failure exits nonzero with a fixed action to unlock the
keyring or provision systemd-creds; it never creates a fallback key or plaintext.
The informational loginctl probe runs once in a separate execution context with
null stdin, rlimits/PDEATHSIG, a two-second deadline, explicit kill/reap and a cached
unknown result on failure. It never runs inside the supervisor loop.

The pinned Windows ONNX DLL is unchanged. Linux pins one runtime: `lib/libonnxruntime.so`
(27,026,609 bytes, SHA-256 `4b3607ae...b566e`) from the sherpa-onnx v2.0.1 linux-x64 shared
no-TTS release (tarball 25,289,904 bytes, SHA-256 `b3b095f6...67b2`). `asr::initialize`
reads the file once, checks size and hash, copies those bytes into a sealed `memfd`
(no write, grow, shrink or further seals), `dlopen`s `/proc/self/fd/N`, and only then
probes `OrtGetApiBase` and API 22. Any other `libonnxruntime.so` is refused with
`ONNX Runtime integrity check failed`. The service needs `/proc/self/fd` and `memfd_create`;
neither has been checked inside the service's private namespace yet. The library is not
shipped or fetched by the server; the operator supplies an absolute path. The load is
proved on the test box through `asr::initialize` and one Silero frame (class
`asr::onnx::linux_tests`). Silero VAD (`Silero::load`, `Silero::push`) and Anuvani language
ID (read once, hashed, loaded from those same bytes) now run on Linux with this runtime;
the test process high-water mark was VmHWM 462,984 kB after one load and five seconds of audio. Batches and the Hindi
child still carry Windows-only gates, so Linux transcription stays unavailable until those lanes land.
`sidenote --cpu-models verify ID --runtime-library /abs/libonnxruntime.so` prints the
version the library reports (`runtime=onnxruntime-<version>-linux-x64-api22 loaded`);
without the flag the status line says `not-loaded` and names the pin.

## Install a user service with headless credentials

Build and install as the unprivileged service owner. The installer maps the current
username to its numeric UID and installs a system-manager drop-in for
`user@UID.service` with After/Wants on `sidenote-credentials@UID.service`.
It starts the broker before starting the current session's Sidenote; it does not
restart an already running user manager or interrupt other user services.

```sh
RUSTUP_TOOLCHAIN=stable cargo build --release --locked --no-default-features --features server --target x86_64-unknown-linux-gnu
owner=$(id -un)
uid=$(id -u "$owner")
sudo install -d -m 700 /etc/credstore.encrypted
# Run once for a new identity. Do not replace a credential used by existing state.
sudo test ! -e "/etc/credstore.encrypted/sidenote-$uid" || exit 1
head -c 32 /dev/urandom | sudo systemd-creds encrypt --with-key=host --name=sidenote-key - "/etc/credstore.encrypted/sidenote-$uid"
sudo chmod 600 "/etc/credstore.encrypted/sidenote-$uid"
bash tools/install_linux_user_service.sh target/x86_64-unknown-linux-gnu/release
```

Both units use Type=exec, so ordering includes credential/exec setup. Both disable
permanent start-rate exhaustion with StartLimitIntervalSec=0 and retry on failure.
A delayed or failed broker therefore cannot permanently strand the consumer after
five attempts. The explicit user@UID dependency is required for lingering boot;
ordering against systemd-user-sessions.service alone is insufficient.

The root-managed credential broker is for systemd 255, whose user manager cannot directly
decrypt host-key credentials. It decrypts into systemd's private runtime credential store
and the user manager copies the credential into the service's private runtime store.
The broker orders the mapped user manager explicitly; linger lets the user manager start at boot
and survives logout/screen-off. On newer systemd versions with user-scoped encrypted
credentials, `LoadCredentialEncrypted` in the user unit can replace this broker after the
owner qualifies that configuration. `--with-key=host` protects disk copies with the host
key; root or a full-disk compromise can recover it. TPM sealing can strengthen that policy,
but was not qualified here. Use encrypted disks for stronger offline protection.

The service restarts on failure, limits its main cgroup to 1 GiB and one CPU, caps tasks,
stops its cgroup on shutdown, and logs `linger=enabled|disabled|unknown` along with fixed
service status. To use Hub add `ExecStart=` then `ExecStart=%h/.local/bin/sidenote --server
--device-role hub` in a user drop-in. Neither role loads UI/GPU dependencies in a server
build. The service's identity is persistent independently of relay enablement.

Backup the encrypted credential, the host credential key through an approved secure backup,
and local replica together. Losing the master key requires re-enrollment; changing it
without re-encrypting existing envelopes makes the identity unavailable. Stop both services
before restoring a matched backup. To uninstall: disable/stop the user service, then the
credential broker and remove its user@UID.service drop-in followed by systemctl daemon-reload. Disable linger only if no other user service needs it. Keep identity and
credentials until the owner has decided on recovery; uninstall never deletes them.

The synthetic process proof is `tools/prove_linux_server.py target/x86_64-unknown-linux-gnu/debug/sidenote`.
It uses temporary data and XDG directories, unsets both display variables, checks
`/proc` mappings and device handles, measures RSS, checks role persistence and duplicate
exclusion, and sends a termination signal. It does not run the repository's test suites.
