# Sidenote 0.0.1

Recording requires Hex. Built-in meeting notes require a separately installed
Claude Code helper and your consent. Without a helper, recording and personal
notes remain available.

## Known limits

The following remain for 0.0.2:

- Claude Code still loads the user's CLAUDE.md and hooks. Sidenote disables its
  tools and MCP configuration, but does not isolate those user customizations.
- OpenCode stores sessions. It is detected but is not offered as a notes helper
  in 0.0.1; Codex is also detected but not offered.
- npm-only Claude Code installations are not detected.

- A running summary cannot be cancelled in 0.0.1.
