# Code map

Where each part of Sidenote lives. Unit tests sit next to the code they test.

| Path | What it owns |
| --- | --- |
| `src/main.rs`, `src/desktop.rs`, `src/lib.rs` | Process entry, command-line modes, and desktop launch. |
| `src/ui/` | The window, built with Rune: sidebar, meeting list, meeting page, live recording page, Ask, Settings. |
| `src/hex_engine.rs`, `src/hex_engine/` | Client for the running Hex app: status, start and stop recording, settings, vocabulary, audio retention. |
| `src/hex_install*.rs`, `src/hex_probe.rs`, `src/hex_labs.rs` | Installing Hex (Windows), detecting it, and Hex compatibility data. |
| `src/listener.rs`, `src/listener/`, `src/core/recording_source*`, `src/core/meeting_watch.rs` | Background work: noticing finished recordings, catch-up, and queue drains. |
| `src/meeting_icon.rs`, `src/meeting_icon/` | The floating meeting indicator (Windows). |
| `src/core/meetings.rs`, `src/core/paths.rs`, `src/core/persistence.rs`, `src/core/store_io.rs` | Meeting folders, data locations, and file reads and writes. |
| `src/core/notes/` | The in-app notes engine: queue, prompt, transcript, translation, status. |
| `scripts/meeting-pipeline/` | The PowerShell and Python notes pipeline (the "scripts" notes engine on Windows). |
| `src/core/meeting_router/`, `src/core/asr/`, `src/core/lid/`, `src/core/speech_*` | Experimental speech routing, the optional local recognition runtime (`asr-runtime` feature), language detection. |
| `src/core/ask*.rs`, `src/core/runners.rs`, `src/core/runner_process*`, `src/core/provider.rs`, `default-runners.json` | Ask: agent CLI runners and sessions, and OpenAI-compatible endpoints. |
| `src/core/tasks*.rs`, `src/core/memory*`, `src/core/daily_check.rs`, `src/core/areas.rs`, `src/core/people.rs`, `src/core/companies.rs` | Tasks, memory, daily checks, projects, people and companies. |
| `src/mcp.rs`, `src/mcp/`, `src/core/agent_tools.rs` | The MCP server. Tool operations live in `agent_tools` so other transports can share them. |
| `src/sync_owner.rs`, `src/sync_owner/`, `src/core/sync_state.rs`, `src/core/relay_preview*` | App side of sync: folder journals, the replica owner, the opt-in relay preview. |
| `crates/sidenote-sync/` | The sync engine as its own crate: protocol, storage, encryption, workers. |
| `crates/meetings-domain/` | Storage-independent domain records (MIT or Apache-2.0). |
| `src/service.rs`, `src/service/` | Device roles (full, server, hub) and the headless service, including the speech service. |
| `src/platform.rs`, `src/platform/` | The OS boundary: Windows and Linux code, single-instance lock, running helper programs. |
| `src/updates*.rs`, `src/update_state.rs` | Windows updates through Velopack. |
| `packaging/` | Linux server install script and systemd units. |
| `relay/` | The sync relay: a Cloudflare Worker in TypeScript, with Bun tests and a local harness. |
| `tools/` | Build, release, fixture and check scripts. Some are only for the maintainer's release process. |
| `tests/`, `tests/fixtures/` | Integration tests and synthetic fixtures. |
| `examples/` | Small programs for probes and measurements. |

`docs/architecture/` is not a map of the current code. It holds dated decision records
(ADRs) and spikes; some describe plans that are not built yet. Read them for the
reasons behind a design, and this page for where the code is today.
