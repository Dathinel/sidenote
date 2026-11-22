# Meeting job contract implementation plan

Base: integration/dev28-next (8407e2d). Scope is retirement stage 1 only.

1. Shared contract: reuse the notes helper failure type, name the existing pipeline stages,
   and define one versioned wire contract for Rust, PowerShell and Python. Synthetic contract
   tests cover vocabulary and restart policy. No new discovery or inference behavior.
2. Engine adoption: reuse `.notes-engine.lock` and OS handle lifetime; persist built-in stages;
   consume the shared contract in scripts; preserve completed summaries on automatic restart.
   Explicit notes-only replay keeps its existing transcript/backup behavior.
3. Failure UI: parse only bounded, allowlisted script diagnostics, expose fixed plain-language
   causes in the live meeting page and floating indicator, and retain existing built-in states.
   Synthetic tests cover hostile/malformed diagnostics and summary precedence.
4. Boundary receipt: refresh the inventory and enumerate every spawn/read site in touched
   source files, old to new, including unchanged expressions whose positions moved.

Each code layer stays at or below 300 changed lines including tests and builds independently.
Validation: wrapper-only fmt and clippy (default and asr-runtime, warnings denied), with this
worktree's target directory; boundary self-test. Unit/concurrency tests are written for release
check, never run on this laptop. No GitHub writes, owner data or discovery changes.
