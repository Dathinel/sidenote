# Sidenote: local-first meeting notes on top of Hex

Sidenote is a local Windows desktop companion to [Hex](https://github.com/anomalyco/hex).
Browse meeting summaries and transcripts, ask questions with your own agent CLI,
and review optional tasks, projects, memory and daily alignment reports.

You need Windows, Hex, and an installed agent CLI such as Claude Code
or Codex for Ask. Configure a provider and model you can use. Provider calls may
send meeting text to that provider. Reading saved meetings does not require an agent.

## Build

Install Rust (stable), the `x86_64-pc-windows-gnu` target and a full MinGW GNU
toolchain including `as.exe` and `dlltool.exe`. Put its bin folder on PATH or set
`RUST_GNU_BIN`, then run from this checkout:

```powershell
./build.ps1
./build.ps1 -Release
```

The executable is `sidenote.exe`, under `target/x86_64-pc-windows-gnu/debug/` or `release/`.
The checked-in Cargo configuration selects Windows GNU, rust-lld and static CRT.

## Use and configuration

The default meeting folder is `%APPDATA%/hex/meetings`; local state and
`config.json` live in `%APPDATA%/sidenote` on new installs. If only the legacy
`%APPDATA%/hex-meetings` folder exists, Sidenote keeps using it in place. Exports
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
