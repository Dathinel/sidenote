# Local Sidenote MCP server

Run `sidenote.exe --mcp` from an MCP client. This is local-only and read-mostly.
It starts before Rune, the GPU, the window single-instance guard and the listener.
The window and listener may remain open. It does not call Hex's local API, read its
discovery credential, run Ask, start a daily check or access the network.

The client launches one process with piped stdin/stdout. Only newline-delimited
JSON-RPC goes to stdout. Diagnostics go to stderr. No port, token or hosted service
is required. Connected apps can read your meetings, typed notes, tasks and memory through this
connection, and send what they read to their AI provider. Meeting text is data, not tool instructions.

## Setup

Open **Settings > Connected apps** for installed clients, their connection status,
an Add button for each, and **Add to all installed**. Show all reveals missing
clients, and selecting a row reveals copy-ready setup text. Installed builds use
their own runtime path, including custom install locations. Keep that executable
at the configured path or reconnect with the installed copy after installation. The usual per-user
installed path is `%LOCALAPPDATA%\sidenote\current\sidenote.exe`, but the page does
not assume that location. Clients may need to reconnect after Sidenote updates.

Add to Claude Code finds the native installed `claude.exe` on PATH or in
`%USERPROFILE%\.local\bin`. It invokes the executable directly with separate
arguments, without a shell, and adds a stdio server in user scope. It first checks
`claude mcp get sidenote`, refuses to replace an existing entry, checks the add exit
status, and checks the entry again afterwards. A failed or inconclusive precheck
stops without adding. Commands run off the UI thread with a 30-second timeout per
step. The page reports success, collision, missing CLI, failure or timeout without
showing potentially private CLI config output. To undo a successful add, run
`claude mcp remove --scope user sidenote`. Real CLI registration remains a manual
integration check.

Codex uses the same check, add and recheck pattern through its own CLI. Its add
command is `codex mcp add sidenote -- <executable> --mcp`. To undo, run
`codex mcp remove sidenote`.

Claude Desktop, Cursor and OpenCode add through their JSON config files. An
existing `sidenote` entry is kept. Comments, invalid JSON or an invalid MCP section
stop the edit and leave the copyable config available. Before changing an existing
file, Sidenote saves its original bytes to `<file>.sidenote-backup-<unix ms>`.
It merges only the new entry, saves through a temporary file in the same folder,
then rereads the entry. Other values survive; JSON key order may change because
`serde_json` does not enable `preserve_order`. The result gives the backup path
to restore. New files contain only the Sidenote configuration and report how to
remove the entry. Add to all runs clients in sequence and reports each result,
continuing after a failure. Claude Desktop is hidden on Linux.

| Client | Config location | Setup |
| --- | --- | --- |
| Claude Code | `%USERPROFILE%\.claude.json` (user scope) | Add button, or copy a PowerShell command. |
| Claude Desktop | `%APPDATA%\Claude\claude_desktop_config.json` | Add button, or copy `mcpServers` JSON. |
| Codex | `%CODEX_HOME%\config.toml`, or `%USERPROFILE%\.codex\config.toml` | Add button, or copy a `[mcp_servers.sidenote]` TOML block. |
| Cursor | `%USERPROFILE%\.cursor\mcp.json` | Add button, or copy `mcpServers` JSON. |
| OpenCode | `%USERPROFILE%\.config\opencode\opencode.jsonc` if present, otherwise `opencode.json` | Add button for strict JSON, or copy local `mcp.sidenote` JSON. Project or managed settings may override it. |

The locations displayed in the app are expanded for this user. Restart or reconnect
the client after setup. This local stdio connection has no network listener, ports,
tokens or hosted sign-in. Web and mobile cloud connectors cannot launch it directly.

For the examples below, replace `C:\Apps\Sidenote\sidenote.exe` with the path shown
in Settings. The GNU debug
build is at `target\x86_64-pc-windows-gnu\debug\sidenote.exe` in the worktree.
Keep the executable available at the configured path after rebuilding.

Claude Code:

```powershell
claude mcp add --transport stdio --scope user sidenote -- "C:\Apps\Sidenote\sidenote.exe" --mcp
```

Claude Desktop, in `%APPDATA%\Claude\claude_desktop_config.json`, uses the same
`mcpServers` JSON shown for Cursor below.

Codex, in `~/.codex/config.toml` (or `$CODEX_HOME/config.toml`):

```toml
[mcp_servers.sidenote]
command = 'C:\Apps\Sidenote\sidenote.exe'
args = ["--mcp"]
```

Cursor, in `.cursor/mcp.json` or the global MCP configuration:

```json
{
  "mcpServers": {
    "sidenote": {
      "command": "C:\\Apps\\Sidenote\\sidenote.exe",
      "args": ["--mcp"]
    }
  }
}
```

OpenCode, in `opencode.json`:

```json
{
  "$schema": "https://opencode.ai/config.json",
  "mcp": {
    "sidenote": {
      "type": "local",
      "command": ["C:\\Apps\\Sidenote\\sidenote.exe", "--mcp"],
      "enabled": true
    }
  }
}
```

Restart or reconnect the client's MCP session after changing configuration. Start by
asking it to search Sidenote for a topic, then read one result's summary or transcript.

## Data folders

These overrides apply to the whole executable, including the window, daily check
and MCP early paths. Pass flags as separate arguments. Flags take precedence over
environment variables. Empty environment values are ignored.

| Folder | Flag | Environment | Default |
| --- | --- | --- | --- |
| App state and device journals | `--data-dir` | `SIDENOTE_DATA_DIR` | `%APPDATA%\sidenote` |
| Local Hex meeting text | `--meetings-dir` | `SIDENOTE_HEX_DIR` | `%APPDATA%\hex\meetings` |
| Peer meeting folders | `--peers-dir` | `SIDENOTE_PEERS_DIR` | `%APPDATA%\sidenote-peers` |

`SIDENOTE_STATE_DIR` remains a fallback alias for the data folder. Every
`SIDENOTE_*` environment name accepts its old `HEX_MEETINGS_*` name too. For each
suffix, a nonempty Sidenote value wins over its old alias. DATA_DIR wins over
STATE_DIR. Flags win over both. Benchmark and render controls accept the aliases too.

For state and peers independently, if the new folder does not exist and the old
`%APPDATA%\hex-meetings` or `%APPDATA%\hex-meetings-peers` exists, Sidenote uses
that old folder in place. It never moves or copies it. If both exist, the Sidenote
folder wins; use `--data-dir` or `--peers-dir` to keep selecting the legacy folder.
Exports follow the selected state root. Background logs use `<state-root>\sidenote.log`
unless the old `%LOCALAPPDATA%\hex-meeting-watch.log` exists and the new log does
not. Explicit data overrides also isolate logs under that root. Existing client registration names
such as `meetings` still work when their command points at an available executable.
See [rename-migration.md](rename-migration.md) before updating a live install.

With an explicit data folder and no peer override, peers default to `<data-dir>\peers` to keep an isolated
instance from reading the owner's normal peer data. The meetings folder is independent;
set both flags to isolate an instance completely. A directory need not exist yet.

```powershell
sidenote.exe --mcp --data-dir "D:\Meetings\state" --meetings-dir "D:\Meetings\recordings" --peers-dir "D:\Meetings\peers"
```

Use these same flags on the window if you want it to review proposals in a custom
data folder. Each installation owns its `device.json`; do not copy that file to a
different machine. Only canonical text meeting files and supported app state are read.
Folder overrides do not relocate Hex itself, its credentials, or the listener's external
pipeline script. See [sync.md](sync.md) for peer layout and journal ownership.

## Tools

Every tool has an input JSON Schema, an outputSchema, a human title, and explicit
annotations. All tools are idempotent, non-destructive, and closed-world. Eight tools
are read-only. `propose_memory` is the sole write tool.

| Tool | Arguments | Use and result |
| --- | --- | --- |
| `search_meetings` | `query`, optional `limit`, `from`, `to`, `project`, `device`, `cursor` | Literal case-insensitive full-text search over titles, summaries, decisions, original AI to-dos, current merged tasks, typed notes and complete transcripts. Metadata and matching snippet, newest first. |
| `list_meetings` | Optional `from`, `to`, `project`, `device`, `limit`, `cursor` | Browse metadata, stable id, title, date, device, project and summary URI. |
| `get_meeting` | `id`, optional `parts`, `offset`, `limit` | Select `summary`, `decisions`, `tasks`, `notes`, `transcript`, `participants`, `metadata`. Defaults to the first three plus metadata. Typed notes and transcript are opt-in and paged. The summary labels its original AI to-dos; read `tasks` for current work. |
| `list_tasks` | Optional `status`, `mine`, `project`, `meeting_id`, `limit`, `cursor` | `status` accepts `todo`, `waiting`, `review`, `done`, `open` (default, every unfinished status), or `all`. `mine=true` selects explicitly marked Mine. Returns current title, description, subtasks with effective ticks, waiting-on link and resolved label, resolved owner/team, due/project edits, manual tasks and source device. Applies the app's normal cleared-task cutoff. |
| `get_memory` | `scope`, optional `query`, `limit`, `cursor` | `company` reads the same legacy-or-accepted company view as the window; `records` includes non-forgotten record statuses and sources; `proposals` includes pending, accepted and rejected status. Proposals are not facts. |
| `list_projects` | Optional `limit`, `cursor` | Current names after aliases, including projects from peer meetings without to-dos. |
| `list_teams` | Optional `limit`, `cursor` | Defined team names, members and owner aliases, paged by name. Available when Tasks is on. |
| `get_daily_check` | `date`, optional `offset`, `limit` | Reads an existing merged report for `YYYY-MM-DD`. `data.report` is a byte page of JSON text. Does not generate a report. |
| `propose_memory` | `text`, `reason`, optional `source_meeting_id` | Queues a reviewable addition. Returns proposal id/status. Does not approve, edit or remove records. |

Successful results have `data`, `warnings`, `notes`, and `truncated`. JSON is returned
as `structuredContent` and in text content for clients that ignore structured output,
preceded by a short summary. Errors use `isError: true` and an actionable text message.
Protocol errors are reserved for wire problems and resource lookup failures.

Collection results use `data.items`, `data.hasMore` and `data.nextCursor`. Reuse the
opaque cursor with exactly the same filters. `limit` defaults to 20 and is capped at
50. Cursors follow the last returned item (the meeting-scoped stable id for v2 tasks), so newly inserted items before it do not
shift the next page. If that item disappears, restart the listing. Data is refreshed
per call, not frozen into a session snapshot. Meeting ids are the app's ids; local
ids take precedence over peer collisions exactly as in the window.

Dates are RFC 3339 with a time-zone offset when the source has a timestamp, otherwise
null. Date filters accept inclusive `YYYY-MM-DD` local calendar dates or RFC 3339
instants with a zone. Unknown dates do not match a date filter. Report dates and due
dates remain calendar dates rather than fabricated timestamps.

Typed-notes, transcript and report offsets are UTF-8 byte offsets, with up to 20,480 bytes per page.
Use `nextOffset` and `hasMore`, and do not calculate a character offset yourself.
Structured results are capped at 96 KiB. Other response text is bounded too, including
unusually large legacy memory fields.
`truncated` and `notes` disclose abbreviation. Narrow filters or select fewer parts
when a response is capped. No meeting audio, arbitrary filesystem files, credentials,
or Ask history is returned by these tools.

Task output uses `status` values `todo`, `waiting`, `review`, `done`, and a
`status_label` matching the app: To do, Waiting on, Review, Done. Completing a linked
blocker makes a waiting task effectively To do, just as in the Tasks page. The
`waiting_on.task_id` is the source stable task id within the same `meeting_id`;
`waiting_label` is the resolved blocker title or free text. A completed blocker may
leave the stored link present while its effective label is null. `stable_id` is the
v2 source id and survives changes to task text; legacy tasks without a source id
return an empty string. Join using `(meeting_id, stable_id)`. The existing `id` and
`task` fields remain for compatibility; the old content-derived `id` can change
when text changes. Manual tasks keep their global stable id and have no real meeting.
Owner aliases resolve to the team's member spelling in the Tasks page and MCP.

Both `list_tasks` and `get_meeting(parts=["tasks"])` use the Tasks page's cleared-before
cutoff. Clearing is a view cutoff, not deletion: old meeting summaries, typed notes
and transcripts remain readable and searchable. The raw `summary.todo` is preserved
and explicitly labelled by `summary.todo_label` as the **original AI list**. It can
contain old task text, ownership and status. Read the merged `tasks` part for current
work. Search includes both this historical summary and effective task values, as
well as typed notes; it does not rewrite the source summary.

## Resources and workflow

Search/list replies include `meeting://<id>` summary URIs and resource links for up to
five results. `resources/templates/list` advertises `meeting://{id}`;
`resources/read` returns the bounded summary, decisions and metadata as JSON text.
Use `get_meeting(parts=["transcript"])` for the transcript rather than putting every
transcript in search results. Optional MCP prompts are omitted; the server's
`instructions` describe the search, read, cite and propose workflow.

## Merged state and concurrency

`core::agent_tools` owns operations independently of `src/mcp.rs`, which handles MCP
schemas, results and stdio. Reads reuse `scan`, `Meeting::search_hit`, `build_tasks`,
project and Teams aliases, memory rendering, daily-report loading and the sync journal merger.
Peer meetings retain their device labels. Local meetings use the installation's name.
Unknown dates and Mine values stay unknown.

MCP reads use the merger's read-only mode. It applies unimported legacy seeds in
memory using the same parser and register rules, without creating device identities,
locks, migration backups, journals, exports or caches. Readers may see independent
store revisions during an active write; immutable journal publication avoids a
partially written transaction. A malformed peer journal produces a warning and is
skipped; unreadable own journals make the operation fail rather than silently reset.

`propose_memory` uses `MemoryStore::stage_agent_proposal`, the existing local writer lock,
and immutable per-device journal publication. The content-derived proposal id makes
retries idempotent. A supplied meeting must exist. An uncited submission gets an
empty source list and an explicit uncited MCP label in its rationale. It is never
presented as meeting evidence. Automated daily-report ingestion still requires
sources; only an explicit owner acceptance can confirm an uncited claim. The existing
proposal format stays readable by older app builds, although accepting an uncited
proposal needs the updated app. Forgotten-source rules
still apply, and a previously rejected proposal is not reopened by retrying it.
The user can reject a pending proposal in Sidenote. Legacy migration may run as part
of this explicit write, using the existing backup-before-import path.

Writer-lock contention returns an error; retry after the window's edit completes.
There is no competing listener subscription or global window mutex. Multiple local
clients may read concurrently. Coexistence with the owner's running window/listener
and proposal review in the GUI still need supervised verification.

## Design notes

A short web research pass on 7 November 2026 checked the current spec, published SDK
and established meeting/memory servers. These are design inputs, not compatibility
tests against those products.

- The [current MCP revision is 2026-07-28](https://modelcontextprotocol.io/specification/2026-07-28).
  It uses per-request metadata and `discover`; `initialize` selects the legacy
  lifecycle. [rmcp 3.9.0](https://crates.io/crates/rmcp/3.9.0) was the published version
  returned by Cargo. Its [official SDK](https://github.com/modelcontextprotocol/rust-sdk)
  handles both lifecycles. We use its `server` and `transport-io` features, with default
  features disabled, and Tokio. This avoids maintaining protocol negotiation and
  JSON-RPC ourselves; no HTTP, TLS or OAuth stack is enabled. The GNU target gate
  checks whether it fits this app's toolchain.
- [Granola](https://www.granola.ai/blog/granola-mcp-claude-chatgpt-cursor),
  [Fireflies](https://guide.fireflies.ai/articles/3039542843-learn-about-fireflies-mcp-server-connect-your-ai-tool?lang=en),
  [Otter](https://help.otter.ai/hc/en-us/articles/35287607569687-Otter-MCP-Server), and
  [Limitless](https://www.limitless.ai/developers/docs/api) informed the meeting-first
  search and retrieve workflow. Search returns compact discovery results; full
  transcripts are a separate opt-in read. We keep provenance and timestamps with
  the result instead of adding a second model-generated answer layer.
- [Screenpipe](https://docs.screenpi.pe/mcp-server) informed local stdio setup across
  coding clients and time-bounded retrieval. We expose text on disk directly instead
  of requiring the desktop window or an authenticated local HTTP API to be running.
- [Basic Memory](https://docs.basicmemory.com/reference/mcp-tools-reference) informed
  project scoping, compact discovery and URI-based reading. Its broader note editing
  and graph traversal are outside this version's scope.
- [OpenMemory](https://mem0.ai/blog/introducing-openmemory-mcp) and
  [Mem0's tool reference](https://github.com/mem0ai/mem0/blob/main/docs/platform/mem0-mcp.mdx)
  informed shared memory across clients, stable identities and filtered retrieval.
  Sidenote adds a proposal review boundary instead of exposing their direct memory
  update/delete operations. No vector store, embedding model or external key is needed.
- The community [Obsidian MCP server](https://github.com/MarkusPfundstein/mcp-obsidian)
  and [Notion's search/read tools](https://developers.notion.com/guides/mcp/mcp-supported-tools)
  informed distinct search and fetch operations over existing notes. We keep the
  tool set task-shaped and bounded, without exposing arbitrary paths or bulk writes.
- The [tools specification](https://modelcontextprotocol.io/specification/2026-07-28/server/tools)
  informed annotations, titles, input/output schemas, structured results, resource
  links and tool-level errors. [Stdio transport rules](https://modelcontextprotocol.io/specification/2026-07-28/basic/transports/stdio)
  inform stdout hygiene. Legacy clients can still initialize with `2025-11-25`;
  current clients use the SDK's discovery and per-request metadata support.

## Verification boundary

The smoke check uses isolated temporary state, meeting and peer fixtures. Build with
`cargo build --locked`, then run `python tools/mcp-smoke.py`. It exercises initialize,
all nine tool schemas and annotations, search, list_tasks, get_meeting and list_teams.
It checks current task fields, all six status filters, owner and project aliases,
stable ids and waiting links, manual tasks, team and task pagination, typed-notes
search and UTF-8 paging, historical summary labelling, cleared-task cutoffs, peer
device labels, disabled-feature behavior and unchanged fixture files. It does not open the GUI or touch the
owner's data or client configs. The transcript is `artifacts/mcp-smoke.json`.

Required local gates for this change: `cargo fmt --all --check`,
`cargo clippy --workspace --locked --all-targets -- -D warnings`,
`cargo clippy --workspace --locked --all-targets --features asr-runtime -- -D warnings`,
`cargo test --workspace --locked`, and
`cargo test --workspace --locked --features asr-runtime`. Unit tests cover effective
fields, status filters, Teams aliases/paging and typed-notes search/paging. Offscreen
Settings scenes are `mcp-connect` and `mcp-connect-560` in `tools/render_ui.ps1`,
rendered in light and dark; they do not launch the desktop window.

On 10 November 2026, all five local Cargo gates above passed. Each test build
(default and `asr-runtime`) ran 163 tests with zero failures. The extended smoke
passed for all nine schemas/annotations and the new task, Teams and typed-notes
behavior, including one-item pages with duplicate task titles and feature-disabled
calls. Its existing-config fixture includes the config reader's persistent lock
file, and every fixture file is checked for changes. The four Connect AI apps
renders at widths 1320 and 560, light and dark, were opened and inspected.
This is test/protocol and offscreen-render evidence. A packaged install's path,
actual clipboard interaction, a real `claude mcp add` run, and connections from the
five AI clients remain unverified. No app was installed or opened on the desktop.

On 7 November 2026, `cargo clippy --locked --all-targets -- -D warnings` and
`cargo build --locked` passed on the configured Windows GNU target. The single
fixture smoke passed with empty stderr, JSON-RPC-only stdout and unchanged fixture
files. No tests or GUI were run. The fixture exercised the legacy initialize path;
current-revision discovery, the other tool executions, real journal concurrency,
proposal acceptance and client-by-client integration remain unverified in live use.

## Optional features

Open Settings > Features in Sidenote to choose the tools available on this device.
Meetings, notes and Ask stay available. A fresh install starts with all optional features
off. An existing install enables matching features once from its saved data when
`config.json` does not exist yet.

| Feature | Tools available when enabled |
| --- | --- |
| Tasks | `list_tasks`, `list_teams` |
| Memory | `get_memory`, `propose_memory` |
| Daily check | `get_daily_check` |
| Projects | `list_projects` |

`tools/list` reads the current config on every request. Calling a disabled tool returns
an error naming the feature and Settings > Features. Clients that cache the tool list
may need to refresh it. Meeting to-dos remain readable through `get_meeting`.
Other devices controls the sidebar Devices section; peer meeting data remains readable.

The small, versioned `config.json` lives at the data directory selected by the existing
folder flags or environment variables. It is device-local, outside synced `state/**`.
The first MCP request may create this config from existing data, including when the
server is the first process launched after an update. Config writes use a flushed temp
file and an atomic rename. An unreadable config is preserved; optional calls fail with
a settings error until it is repaired. No MCP tool changes feature settings.

Feature switches change what the app shows and which dedicated tools are advertised.
They are **not access controls**: meeting tools and meeting resources can still
expose tasks in summaries and the merged tasks part, project metadata, and meetings
from other devices when those UI features are off. Clearing tasks is also not a
privacy boundary. Use only AI apps you trust with the meeting content.
