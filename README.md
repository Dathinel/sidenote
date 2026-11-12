# Sidenote: local-first meeting notes on top of Hex

Sidenote is a local Windows desktop companion to [Hex](https://github.com/anomalyco/hex).
Browse meeting summaries and transcripts, ask questions with your own agent CLI,
and review optional tasks, projects, memory and daily alignment reports.

You need Windows, Hex, and an installed agent CLI such as Claude Code
or Codex for Ask. Configure a provider and model you can use. Provider calls may
send meeting text to that provider. Reading saved meetings does not require an agent.

## Install and updates

Download `sidenote-win-Setup.exe` from the latest stable
[GitHub Release](https://github.com/harshanandak/hex-meetings/releases/latest).
Run Setup to install Sidenote for your Windows user. The 0.0.1 installer is
**unsigned**: Windows may show "Windows protected your PC". After checking that
the file came from that release, choose **More info**, then **Run anyway**.
If your organisation blocks unsigned applications, ask its administrator to
approve the installer; that policy may prevent the override.

Installed copies check GitHub Releases when opened, every six hours, and when
the window gets focus if the last attempt was over 30 minutes ago. Updates
download in the background. Use **Settings > Updates > Install and restart**,
or keep working and install the pending update on the next launch. Installation
waits for recording and background jobs to finish. A manually copied executable
does not update. `SIDENOTE_UPDATE_FEED` overrides GitHub with a local development
folder; clear it for public updates. See [release and update details](docs/updates.md).

The notes pipeline still needs separately installed helpers. On a PC without
them, recording works but Sidenote says "Meeting notes are not set up on this PC".
Meeting notes move into the app in 0.0.2.

If you used the private 0.0.6 build, back up `%APPDATA%/hex-meetings`, uninstall
0.0.6, then install 0.0.1. Clear `SIDENOTE_UPDATE_FEED` or point it at a fresh,
empty folder; the old development feed can otherwise offer 0.0.6 again.

## Build

Install Rust (stable), the `x86_64-pc-windows-gnu` target and a full MinGW GNU
toolchain including `as.exe` and `dlltool.exe`. Put its bin folder on PATH or set
`RUST_GNU_BIN`, then run from this checkout:

```powershell
./tools/local_build.ps1 build --locked
./tools/local_build.ps1 build --release --locked
```

The executable is `sidenote.exe`, under `target/x86_64-pc-windows-gnu/debug/` or `release/`.
The checked-in Cargo configuration selects Windows GNU, rust-lld and static CRT.

## Use and configuration

The default meeting folder is `%APPDATA%/hex/meetings`; local state and
`config.json` live in `%APPDATA%/hex-meetings` by default, including on new installs.
If `%APPDATA%/sidenote` already exists, Sidenote uses that folder instead. Exports
stay under that selected root. New logs use `<state-root>/sidenote.log`; an existing
`%LOCALAPPDATA%/hex-meeting-watch.log` stays in place. Nothing is moved or copied. See the
[owner migration checklist](docs/rename-migration.md) and [MCP setup](docs/mcp.md)
for stdio integration, tools, trust boundaries and directory overrides.
See [private configuration](docs/configuration.md) for optional KB context.
Features are opt-in in Settings. Existing feature choices migrate on first use.
Runner definitions live in device-local `runners.json`; bundled definitions are examples.

The notes pipeline currently requires separately installed local helpers. This
repository does not distribute those private installation scripts. See the
[architecture](docs/architecture/README.md) for planned integration work.

## Privacy and security

Meeting text, memory, exports and sync journals can contain private information.
Keep them outside Git. Agent and MCP clients can read the corpus you configure.
Synced device folders currently require fully trusted peers; checksums do not
authenticate their authors. Tombstones do not erase historical backups or replicas.
See [SECURITY.md](SECURITY.md), [sync](docs/sync.md) and
[audio retention](docs/audio-retention.md).

## Render fixtures

`tools/render_ui.ps1` uses synthetic Northwind meetings and isolated temporary
state. `tools/prepare_check_fixture.py` generates check and memory examples without
reading AppData or calling a model. Render output stays under ignored `artifacts/`.

## Licence

Sidenote is licensed under [AGPL-3.0-or-later](LICENSE).
[Third-party notices and dependency licences](THIRD_PARTY.md) cover Hex, Rune,
bundled fonts and the locked Cargo dependency inventory.

### Local cargo and release work

Run every local cargo command through `tools/local_build.ps1`; it limits concurrent
builds and waits for recent meeting activity. For example:

```powershell
./tools/local_build.ps1 build --locked
```

Use `./tools/local_build.ps1 -Pack` with the parameters in `tools/release_pack.ps1`
for release packing. The meeting activity check is a log-based heuristic.
