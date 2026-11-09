# Notes engine API handoff

E1: `Helper::offered()` (`src/core/notes/helper.rs:19`) returns true only for
Claude Code in 0.0.1. `notes::detect()` still reports all three installations.
The UI owns labels, privacy wording and disabled helper controls.

`NotesSettings::effective_helper()` (`src/core/notes/mod.rs:87`) returns no helper
for saved Codex/OpenCode choices. `NotesSettings::helper_reason()`
(`src/core/notes/mod.rs:91`) returns the fixed code `helper_not_offered` for those
choices. Use the effective helper for selection and the code for UI-owned copy.
Stored choices are retained; neither queue processing nor the CLI test entry point
can run an unoffered helper. Command definitions remain for a later release.

E6: `agent_instructions::Store::using_defaults_after_error`
(`src/core/agent_instructions.rs:26`) reports that invalid/unreadable synced
instructions were replaced by compiled defaults for this read. Files are retained.
`src/ui/ai_notes.rs:254` wires that flag into the existing error banner while still
loading settings. This lane reuses the existing load-error text; the UX lane owns
its wording and placement, along with the E1 helper list and privacy text.

Limits: helper stdout is limited to 4 MiB including structured events, with bounded
line buffering and channel backpressure. JSON output strings are limited to 16,000
characters per field; generated project names to 100. Transcript input retains
its 250,000-character limit. Typed notes and glossary each have a separate 16,000-
character limit, including redo/first-part requests.

E7: `tools/pack_release.ps1:47` requests `--shortcuts StartMenuRoot`, without
Desktop. Checked against the installed vpk 0.0.1298 `--skip-updates pack --help`;
its default lists `Desktop,StartMenuRoot`. `tools/pack_release_proof.ps1` parses
the real argument array, passes it to a fake runner in an OS temp folder, and
checks installed help without executing the release script.

Upgrade limit: the installed vpk's bundled `vendor/update.exe` contains both
`Shortcut Previous Locations:` and `Shortcut Next Locations:` diagnostics and
recognizes `startmenuroot`. That is static evidence of location comparison, not
proof that a dev.2-to-dev.3 update creates a missing shortcut. That migration
requires an authorized installer/update test; packing, installing and app launch
were prohibited in this task, so it remains unverified.
