# Hex engine API

Decisions needed: none.

`sidenote::hex_engine::HexEngine` is the core API for the Sidenote UI. Clone the client to share its in-memory CLI path and capability cache. Hex remains a separate, unmodified process. The client never starts, quits, or restarts it.

## Calls

All methods return `Result<T, HexError>`. Calls block, so run them outside the UI event thread. The only CLI invocations are hidden `hex.com <group> --help` processes with piped output. Application commands use authenticated loopback `POST /control` directly, with protocol 4. The bearer credential stays in memory. There is no shell command interpolation, settings-file editing, transcript reading, or meeting-directory access.

| Method | Return | Hex command or equivalent control request |
|---|---|---|
| `status()` | `EngineState` | `status --json`, wire `status` |
| `record_start(Option<&str>)` | `()` | `meetings record [--title T]`, wire `meeting-record {title?}` |
| `record_stop()` | `()` | `meetings stop`, wire `meeting-stop` |
| `dictation_stop()` | `()` | `dictation stop`, wire `dictation {action: stop}` |
| `dictation_toggle()` | `()` | `dictation toggle`, wire `dictation {action: toggle}` |
| `paste_last()` | `()` | `paste-last` |
| `settings_list()` | `Vec<Setting>` | `settings list --json`, wire `settings-get` |
| `settings_get(key)` | `Vec<Setting>` | `settings get KEY --json`, wire `settings-get {key}`; supports prefixes |
| `settings_set(key, value: &str)` | `()` | `settings set KEY VALUE`, wire `settings-set {key, value}`; Hex validates type, range and writability |
| `vocabulary_list()` | `Vec<VocabularyEntry>` | `vocabulary list --json`, wire `vocabulary-list` |
| `vocabulary_add(term, sounds_like: &[&str])` | `()` | `vocabulary add TERM [--sounds-like a,b]`, wire `vocabulary-add {term, sounds_like}` |
| `vocabulary_remove(term)` | `()` | `vocabulary remove TERM`, wire `vocabulary-remove {term}` |
| `models_list()` | `Vec<ModelInfo>` | `model list --json`, wire `model-list` |
| `model_status()` | `ModelState` | selected runtime model from `status --json`; does not run the separate local checksum tool `model status` |
| `capabilities()` | `BTreeMap<Feature, Support>` | protocol-4 status capabilities plus cached root, meetings, dictation, settings, vocabulary and model help |
| `watch()` | `StatusWatcher` | status polling and existing listener invalidations |

`EngineState` has running, version, protocol, meeting state, optional recording, optional dictating state, capture state and model. A disconnected engine returns `HexNotRunning`, rather than a fabricated idle state. `dictating: None` means unknown. `Recording` exposes optional id/title/since/elapsed seconds. This build does not provide an active meeting id; it remains `None`. `since` is estimated from the host clock and Hex's elapsed seconds. The model reports id/name/language/installed/preparing/downloaded bytes. Missing optional fields stay unknown.

`Setting` preserves value, kind, choices, writable, available, about and any `secret` flag for the settings UI. No returned settings values are logged, including unmarked ones. The caller must keep those values private. Settings, vocabulary, model and status structs have redacted `Debug` output. The client suppresses free-form server messages and all raw stderr. `Failed` contains a fixed diagnostic summary and an allowlisted Hex error code, so failures cannot echo submitted values, meeting titles, history text or tokens. History is never requested.

## Timeouts and compatibility

Status and model-status network calls have a 750 ms total transport budget. Other requests have 12 seconds, including meeting start's documented 8-second acknowledgement. Status uses no subprocess and no help lookup. The CLI path is cached as `%APPDATA%/hex/versions/<reported-app-version>/hex.com`; the version is validated as a path component. No version directory is hard-coded or selected by sorting directories.

The first capability lookup also has a shared 12-second help budget, plus its status call. Subsequent calls reuse the inventory until Hex's version or reported capabilities change. A method needing an inventory can therefore take up to roughly 25 seconds on its first call, including preflight and the operation. Calls use capped HTTP lines, headers, request bodies and response bodies, and accept fixed-length, chunked or connection-close responses. Hidden help children are attached to the existing kill-on-close process job; timeout closes the job and reaps the child. A pending `gpu-draw.pending` marker prevents help invocations.

Errors are `HexNotRunning`, `Unsupported`, `Timeout`, or `Failed { code, stderr_summary }`. Protocol versions other than 4 are conservatively unsupported. Feature states are `Supported`, `Unsupported`, and `Unknown`. A negative platform capability overrides help; successfully read help without a command means unsupported; missing/failed help means unknown. Unknown features are refused by write methods. A runtime `unsupported` response is also typed `Unsupported`. Supported means the build advertises the operation, not that a live write has been exercised or that current settings permit it.

## Visibility and events

Create one watcher for the window, then call `set_visible(true)` on show/restore and `set_visible(false)` on hide/minimize. It starts hidden. Read `updates` on a UI worker and forward results into the UI's normal update mechanism. The one-item channel bounds memory; if a result is pending, new samples are dropped until the UI drains it. Do not keep displaying an old state after an error. Dropping the watcher signals stop and joins the worker, with at most an in-flight 750 ms status request to finish.

While visible, the worker waits on Windows events and polls after 1.5 seconds of silence. While hidden, it waits indefinitely for visibility or stop, and makes no Hex calls. The existing `/meetings/events` listener signals `Local\HexMeetingsEngineChanged` when it connects and after parsing an event. The watcher wakes and refreshes on that signal while visible. It never opens a competing SSE subscription. Polling still works if the listener is absent, old, or the Labs stream is unavailable. Using the event relay across processes requires the listener to run a binary containing this change; this task does not replace or restart the owner's existing listener. The event has no payload and uses the Windows default access control. One watcher is intended per Sidenote window.

## Read-only development command

Run `sidenote.exe --hex-engine-status` with hidden process creation and piped stdout. It exits before the single-instance, GPU and UI paths. It reads ten status samples, the capability inventory, settings list and meetings prefix, vocabulary list, model list, and runtime model status. It prints counts and redacted state only. `--watch-visible` or `--watch-hidden` adds a 20-second watcher sample without creating any window. These flags never call write methods.

Validation follows the owner's rule: no tests and no live write calls. Record, dictation, paste, settings set, and vocabulary mutations require a supervised session later. The wire mappings were reviewed against the [pinned upstream CLI protocol](https://github.com/anomalyco/hex/blob/f5681bcd4179b1d78484da00b96691b8a080d197/docs/cli.md) and the installed build's read-only help. They remain unverified in live use.
## Sidenote UI

`src/ui/hex.rs` holds the engine client's state, status and Hex page; `src/ui/nav.rs` and `src/ui/live.rs` hold the recording flow. The sidebar's bottom line shows the status (Not running, Idle, Recording with elapsed time, Dictating). One "New meeting" button at the top right starts a recording through `record_start(None)` and opens the live page; while Hex records, the top right and the sidebar line show "Recording 12:34" with Stop, which calls `record_stop`. There are no separate Record and Stop pills. Disabled reasons (Hex not running, already recording, build cannot record) are tooltips.

The live page has an editable title, a recording mark, the elapsed time and a large Stop button. Hex gives the client no live transcript, and the client by design never reads the meeting folder, so the page shows "Recording, notes appear when you stop" instead of a rolling transcript. The client has no call to rename a running meeting either, so the title is the owner's own and stays in this window; the notes pipeline names the meeting. After Stop the page says "Making notes" and rescans the meetings folder every 6 seconds until a new local meeting appears, then opens it. A recording begun in Hex itself gets the same Recording pill and, after it stops, the same wait.

The Hex page has a search box over all settings and a section menu: Model (read-only), Vocabulary (list, add, remove), General, Dictation, Meetings, Shortcuts and Advanced. Settings are placed by key prefix (`dictation.`, `meetings.`, `general.`, a shortcut kind or a key with shortcut or hotkey in it). Names drop the prefix, and the one-line description is the first sentence of what Hex says. Advanced holds `labs.*` as "Experimental (Labs)" and, as "Rarely used", anything read only, unavailable, hidden or with a prefix the page does not know. Both groups are collapsed until opened. A set is sent only when the value differs from what Hex reports.

A forwarder thread owns the `StatusWatcher` and tells it when the window is shown or hidden (window focus, plus a minimize check on each sample). Every other call runs on its own thread and returns through the UI's event channel. Errors appear as a short strip or a line under the control, never a dialog. Controls are disabled when Hex is not running or the build lacks the command. Nothing in the UI logs or shows the token or `local-api.json`, and secret settings show as Hidden.

Not exercised live: record start and stop, settings set, vocabulary add and remove. They wait for the supervised session.
