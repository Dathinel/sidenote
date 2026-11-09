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
