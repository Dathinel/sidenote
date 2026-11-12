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
`--target x86_64-pc-windows-gnu` explicitly; the portable root toolchain uses `stable`.

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
Authenticated phone MCP, service installation and continuous transport/pairing work
remain later audit steps.

On Linux, data defaults to `$XDG_DATA_HOME/sidenote` or `~/.local/share/sidenote`.
XDG configuration and state roots have separate helpers. Relative XDG roots are ignored.
On Linux an explicit data directory also isolates default Hex and peer paths.
Windows keeps the dev.19 Hex/captures defaults; set explicit meeting/capture overrides
for isolated Windows work. Explicit meeting and peer directory overrides take precedence.
Linux writes use private files, pinned directory descriptors, no-follow opens, atomic replacement and nonblocking `flock`.

Linux currently returns `UNAVAILABLE_ON_PLATFORM` for legacy PowerShell execution,
updates, native desktop operations, encrypted relay identity and the qualified Windows
ONNX loader. Sync workers retain their unavailable capability until Unix containment
is qualified. Agent executable discovery and process containment fail closed on Linux;
there is no plaintext identity fallback or uncontained processing fallback. Existing
Syncthing file transport remains external. This lane does not establish two-device or
phone acceptance.

The synthetic process proof is `tools/prove_linux_server.py target/x86_64-unknown-linux-gnu/debug/sidenote`.
It uses temporary data and XDG directories, unsets both display variables, checks
`/proc` mappings and device handles, measures RSS, checks role persistence and duplicate
exclusion, and sends a termination signal. It does not run the repository's test suites.
