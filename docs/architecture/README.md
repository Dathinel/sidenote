# Architecture decisions

Status as of 2026-11-08. These records set the shape of Meetings for the next phases:
encrypted personal sync, then server-readable team workspaces with a remote MCP server,
while the app stays open source, offline-first and usable with no account.

| ADR | Decision in one line |
| --- | --- |
| [0001](adr-0001-domain-model-and-storage-seam.md) | Stable UUID ids for every entity and one versioned `Store` API that the UI, MCP, pipeline and daily check use. |
| [0002](adr-0002-sync-engine.md) | Keep device journals as the engine (upgrade to v3); Automerge replaces them only if it passes a fixed pilot gate. |
| [0003](adr-0003-encryption-and-keys.md) | age envelopes under a per-space key, device keys in the OS keystore, a recovery key, and epoch rotation on revoke. Team content enters only by an explicit, labelled copy. |
| [0004](adr-0004-server.md) | One AGPL Rust service on Postgres and S3-compatible storage; identity from an OIDC provider (Better Auth by default, Ory Hydra as fallback); SQL ACL tables. |
| [0005](adr-0005-remote-mcp.md) | rmcp streamable HTTP with MCP OAuth; the same tool catalog as the local server through a shared `meetings-tools` crate. |
| [0006](adr-0006-agent-harness-layer.md) | One `Harness` trait over ACP with a CLI fallback; the notes pipeline becomes a resumable Rust job runner that writes status files. |
| [0007](adr-0007-chat-history.md) | Named chat threads with per-answer provenance, local FTS5 search, JSON and Markdown export, and a `propose_chat_summary` tool. |
| [0008](adr-0008-distribution-and-updates.md) | Velopack installer and updates from GitHub Releases, signed through SignPath Foundation, Azure Artifact Signing as fallback. |

Build order: [ROADMAP.md](ROADMAP.md). Pilot task: [spikes/automerge-pilot-spec.md](spikes/automerge-pilot-spec.md).

Evidence: these records come from reading the code at `8ef57b3`, the review reports of
2026-11-08 and vendor documentation. No code was built, no test ran, and no external
service was tried. Every vendor claim marked "verify" must be checked in the lane that
first depends on it.
