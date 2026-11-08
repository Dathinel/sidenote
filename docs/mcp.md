# Local Meetings MCP server

Run `hex-meetings.exe --mcp` from an MCP client. This is local-only and read-mostly.
It starts before Rune, the GPU, the window single-instance guard and the listener.
The window and listener may remain open. It does not call Hex's local API, read its
discovery credential, run Ask, start a daily check or access the network.

The client launches one process with piped stdin/stdout. Only newline-delimited
JSON-RPC goes to stdout. Diagnostics go to stderr. No port, token or hosted service
is required. Connected agents receive meeting and memory content, so use clients
you trust with that content. Meeting text is data, not tool instructions.

## Setup

Replace `C:\Apps\Meetings\hex-meetings.exe` with your built executable. The GNU debug
build is at `target\x86_64-pc-windows-gnu\debug\hex-meetings.exe` in the worktree.
Keep the executable available at the configured path after rebuilding.

Claude Code:

```powershell
claude mcp add meetings -- "C:\Apps\Meetings\hex-meetings.exe" --mcp
```

Codex, in `~/.codex/config.toml`:

```toml
[mcp_servers.meetings]
command = 'C:\Apps\Meetings\hex-meetings.exe'
args = ["--mcp"]
```

Cursor, in `.cursor/mcp.json` or the global MCP configuration:

```json
{
  "mcpServers": {
    "meetings": {
      "command": "C:\\Apps\\Meetings\\hex-meetings.exe",
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
    "meetings": {
      "type": "local",
      "command": ["C:\\Apps\\Meetings\\hex-meetings.exe", "--mcp"],
      "enabled": true
    }
  }
}
```

Restart or reconnect the client's MCP session after changing configuration. Start by
asking it to search Meetings for a topic, then read one result's summary or transcript.

## Data folders

These overrides apply to the whole executable, including the window, daily check
and MCP early paths. Pass flags as separate arguments. Flags take precedence over
environment variables. Empty environment values are ignored.

| Folder | Flag | Environment | Default |
| --- | --- | --- | --- |
| App state and device journals | `--data-dir` | `HEX_MEETINGS_DATA_DIR` | `%APPDATA%\hex-meetings` |
| Local Hex meeting text | `--meetings-dir` | `HEX_MEETINGS_HEX_DIR` | `%APPDATA%\hex\meetings` |
| Peer meeting folders | `--peers-dir` | `HEX_MEETINGS_PEERS_DIR` | `%APPDATA%\hex-meetings-peers` |

`HEX_MEETINGS_STATE_DIR` remains a fallback alias for the data folder. With an explicit
data folder and no peer override, peers default to `<data-dir>\peers` to keep an isolated
instance from reading the owner's normal peer data. The meetings folder is independent;
set both flags to isolate an instance completely. A directory need not exist yet.

```powershell
hex-meetings.exe --mcp --data-dir "D:\Meetings\state" --meetings-dir "D:\Meetings\recordings" --peers-dir "D:\Meetings\peers"
```

Use these same flags on the window if you want it to review proposals in a custom
data folder. Each installation owns its `device.json`; do not copy that file to a
different machine. Only canonical text meeting files and supported app state are read.
Folder overrides do not relocate Hex itself, its credentials, or the listener's external
pipeline script. See [sync.md](sync.md) for peer layout and journal ownership.

## Tools

Every tool has an input JSON Schema, an outputSchema, a human title, and explicit
annotations. All tools are idempotent, non-destructive, and closed-world. Seven tools
are read-only. `propose_memory` is the sole write tool.

| Tool | Arguments | Use and result |
| --- | --- | --- |
| `search_meetings` | `query`, optional `limit`, `from`, `to`, `project`, `device`, `cursor` | Literal case-insensitive full-text search over titles, summaries, decisions, to-dos and complete transcripts. Metadata and matching snippet, newest first. |
| `list_meetings` | Optional `from`, `to`, `project`, `device`, `limit`, `cursor` | Browse metadata, stable id, title, date, device, project and summary URI. |
| `get_meeting` | `id`, optional `parts`, `offset`, `limit` | Select `summary`, `decisions`, `tasks`, `transcript`, `participants`, `metadata`. Defaults to the first three plus metadata. Transcript is opt-in and paged. |
| `list_tasks` | Optional `status`, `mine`, `project`, `meeting_id`, `limit`, `cursor` | `status` is `open` by default, `done` or `all`. `mine=true` selects explicitly marked Mine. Returns merged owner/due/project edits, ticks, manual tasks and source device. Applies the app's normal cleared-task cutoff. |
| `get_memory` | `scope`, optional `query`, `limit`, `cursor` | `company` reads the same legacy-or-accepted company view as the window; `records` includes non-forgotten record statuses and sources; `proposals` includes pending, accepted and rejected status. Proposals are not facts. |
| `list_projects` | Optional `limit`, `cursor` | Current names after aliases, including projects from peer meetings without to-dos. |
| `get_daily_check` | `date`, optional `offset`, `limit` | Reads an existing merged report for `YYYY-MM-DD`. `data.report` is a byte page of JSON text. Does not generate a report. |
| `propose_memory` | `text`, `reason`, optional `source_meeting_id` | Queues a reviewable addition. Returns proposal id/status. Does not approve, edit or remove records. |

Successful results have `data`, `warnings`, `notes`, and `truncated`. JSON is returned
as `structuredContent` and in text content for clients that ignore structured output,
preceded by a short summary. Errors use `isError: true` and an actionable text message.
Protocol errors are reserved for wire problems and resource lookup failures.

Collection results use `data.items`, `data.hasMore` and `data.nextCursor`. Reuse the
opaque cursor with exactly the same filters. `limit` defaults to 20 and is capped at
50. Cursors follow the last returned id, so newly inserted items before it do not
shift the next page. If that item disappears, restart the listing. Data is refreshed
per call, not frozen into a session snapshot. Meeting ids are the app's ids; local
ids take precedence over peer collisions exactly as in the window.

Dates are RFC 3339 with a time-zone offset when the source has a timestamp, otherwise
null. Date filters accept inclusive `YYYY-MM-DD` local calendar dates or RFC 3339
instants with a zone. Unknown dates do not match a date filter. Report dates and due
dates remain calendar dates rather than fabricated timestamps.

Transcript and report offsets are UTF-8 byte offsets, with up to 20,480 bytes per page.
Use `nextOffset` and `hasMore`, and do not calculate a character offset yourself.
Structured results are capped at 96 KiB. Other response text is bounded too, including
unusually large legacy memory fields.
`truncated` and `notes` disclose abbreviation. Narrow filters or select fewer parts
when a response is capped. No meeting audio, arbitrary filesystem files, credentials,
or Ask history is returned by these tools.

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
project aliases, memory rendering, daily-report loading and the sync journal merger.
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
The user can reject a pending proposal in Meetings. Legacy migration may run as part
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
  Meetings adds a proposal review boundary instead of exposing their direct memory
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

The allowed smoke check uses an isolated temporary data folder, local recordings and
peer fixture. It sends `initialize`, `tools/list` and one `search_meetings` request.
It does not touch the owner's AppData, run tests, open the GUI or exercise a live
memory write. Hosted CI remains the test gate. The commit report records the two
required Cargo gates and the smoke replies.

On 7 November 2026, `cargo clippy --locked --all-targets -- -D warnings` and
`cargo build --locked` passed on the configured Windows GNU target. The single
fixture smoke passed with empty stderr, JSON-RPC-only stdout and unchanged fixture
files. No tests or GUI were run. The fixture exercised the legacy initialize path;
current-revision discovery, the other tool executions, real journal concurrency,
proposal acceptance and client-by-client integration remain unverified in live use.

Run it with `python tools/mcp-smoke.py` after building. It checks every advertised
tool's annotations and schemas, title/transcript search, project aliases, peer device
labels, local-id precedence and unchanged fixture files. The full JSON-RPC transcript
is written to `artifacts/mcp-smoke.json`, outside version control.

## Optional features

Open Settings > Features in Meetings to choose the tools available on this device.
Meetings, notes and Ask stay available. A fresh install starts with all optional features
off. An existing install enables matching features once from its saved data when
`config.json` does not exist yet.

| Feature | Tools available when enabled |
| --- | --- |
| Tasks | `list_tasks` |
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
