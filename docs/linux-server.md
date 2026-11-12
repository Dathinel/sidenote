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
attribute `application=sidenote`, storing a random master key in the default persistent
collection on first use. Cargo.lock contains no libsecret/keyring client crate, so the
small FFI adapter loads libsecret at runtime. A private provider lock serializes initial
creation across processes. No session keyring means an explicit startup failure; no
plaintext or newly generated substitute key is used to decrypt an existing identity.
These calls can prompt the owner to unlock the desktop keyring. For an unattended host,
use the credential provisioning below instead.

Each Linux child receives a new process group, 512 MiB address-space limit, 120 CPU
seconds, core dumps disabled, no-new-privileges and `PR_SET_PDEATHSIG=SIGKILL` before exec.
The parent pid is checked again to close the parent-death race. Sync workers verify the
limits and death signal at entry, use independent budgets, and keep the four-worker pool
limit. Timeouts/drop kill and reap workers. Notes/routing and their CLI helpers use the
same before-exec limits; normal shutdown kills their groups. These are CPU-time limits,
not a CPU percentage, and address space rather than Windows committed-memory accounting.
Long-running model CLIs may need a later, measured budget policy. A helper can create
children outside the Rust adapter: full descendant cleanup after abrupt service death
is supplied by systemd `KillMode=control-group`; the per-process death signal alone is
not a general process-tree sandbox. No privileged cgroup operation is performed by the
application. The user unit requests cgroup memory/CPU limits when its manager is delegated
the controllers; rlimits remain mandatory even without that delegation.

The pinned Windows ONNX DLL is unchanged. No Linux runtime artifact has been qualified
against that pin; `asr-runtime` therefore returns `ONNX_RUNTIME_UNAVAILABLE` explicitly.
Both language-detector and recognition paths report unavailable/degraded output. Arbitrary
`libonnxruntime.so` files are never loaded. Linux ASR execution remains unavailable in
this step, which is one of the goal's permitted outcomes.

## Install a user service with headless credentials

Install the built `sidenote` and adjacent `sidenote-sync-worker` in `~/.local/bin`.
Do not copy identity state from another device. The following owner-run commands provision
a fresh local key without putting it in arguments, terminal output or a plaintext disk file:

```sh
install -d -m 700 ~/.local/bin ~/.config/systemd/user
install -m 755 target/x86_64-unknown-linux-gnu/release/sidenote ~/.local/bin/sidenote
install -m 755 target/x86_64-unknown-linux-gnu/release/sidenote-sync-worker ~/.local/bin/sidenote-sync-worker
sudo install -d -m 700 /etc/credstore.encrypted
# Run exactly once. Never overwrite this encrypted key for an existing identity.
sudo test ! -e "/etc/credstore.encrypted/sidenote-$USER" || exit 1
head -c 32 /dev/urandom | sudo systemd-creds encrypt --with-key=host --name=sidenote-key - "/etc/credstore.encrypted/sidenote-$USER"
sudo chmod 600 "/etc/credstore.encrypted/sidenote-$USER"
sudo install -m 644 packaging/systemd/sidenote-credentials@.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now "sidenote-credentials@$USER.service"
install -m 644 packaging/systemd/sidenote.service ~/.config/systemd/user/sidenote.service
install -d -m 700 ~/.config/systemd/user/sidenote.service.d
printf '[Service]\nLoadCredential=sidenote-key:/run/credentials/sidenote-credentials@%s.service/sidenote-key\n' "$USER" > ~/.config/systemd/user/sidenote.service.d/credential.conf
sudo loginctl enable-linger "$USER"
systemctl --user daemon-reload
systemctl --user enable --now sidenote.service
loginctl show-user "$USER" -p Linger
systemctl --user status sidenote.service
```

The root-managed credential broker is for systemd 255, whose user manager cannot directly
decrypt host-key credentials. It decrypts into systemd's private runtime credential store
and the user manager copies the credential into the service's private runtime store.
The broker starts before user sessions; enable linger so the user manager starts at boot
and survives logout/screen-off. On newer systemd versions with user-scoped encrypted
credentials, `LoadCredentialEncrypted` in the user unit can replace this broker after the
owner qualifies that configuration. `--with-key=host` protects disk copies with the host
key; root or a full-disk compromise can recover it. TPM sealing can strengthen that policy,
but was not qualified here. Use encrypted disks for stronger offline protection.

The service restarts on failure, limits its whole cgroup to 1 GiB and one CPU, caps tasks,
stops its cgroup on shutdown, and logs `linger=enabled|disabled|unknown` along with fixed
service status. To use Hub add `ExecStart=` then `ExecStart=%h/.local/bin/sidenote --server
--device-role hub` in a user drop-in. Neither role loads UI/GPU dependencies in a server
build. The service's identity is persistent independently of relay enablement.

Backup the encrypted credential, the host credential key through an approved secure backup,
and local replica together. Losing the master key requires re-enrollment; changing it
without re-encrypting existing envelopes makes the identity unavailable. Stop both services
before restoring a matched backup. To uninstall: disable/stop the user service, then the
credential broker. Disable linger only if no other user service needs it. Keep identity and
credentials until the owner has decided on recovery; uninstall never deletes them.

The synthetic process proof is `tools/prove_linux_server.py target/x86_64-unknown-linux-gnu/debug/sidenote`.
It uses temporary data and XDG directories, unsets both display variables, checks
`/proc` mappings and device handles, measures RSS, checks role persistence and duplicate
exclusion, and sends a termination signal. It does not run the repository's test suites.
