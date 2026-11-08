# ADR 0005: Remote MCP

Status: accepted, 2026-11-08. ChatGPT and Claude.ai behavior is unverified until lane V2.

## Context

The local server (`src/mcp.rs`, `src/core/agent_tools.rs`) uses rmcp 3.9 over stdio with 8
tools: `search_meetings`, `list_meetings`, `get_meeting`, `list_tasks`, `get_memory`,
`list_projects`, `get_daily_check` and `propose_memory`. It has caps (50 results, 20 KiB text
pages, 96 KiB structured results), cursors and feature gating from `config.json`. Cloud
assistants cannot launch a local process, so team workspaces need an HTTPS endpoint with
OAuth that serves only what the signed-in user may read.

## Decision

**Transport.** rmcp's streamable HTTP server transport, mounted in axum at `/mcp`, at the same
pinned rmcp version as the app (verify the feature name at 3.9). Use stateless JSON responses, so
any server replica can answer. Add SSE streaming only when a tool needs progress.

**Auth, per the MCP authorization spec** (target revision 2026-07-28, compatible with 2025-11-25).
The server is an OAuth resource server:
- **Discovery:** an unauthenticated call gets `401` with `WWW-Authenticate: Bearer resource_metadata="https://<host>/.well-known/oauth-protected-resource"`. That document (RFC 9728) names the authorization server, which publishes RFC 8414 metadata.
- **Flow:** authorization code with PKCE S256. The `resource` parameter (RFC 8707) binds the token audience to `https://<host>/mcp`; tokens for any other audience are rejected.
- **Client registration:** Client ID Metadata Documents first, dynamic registration as fallback, and a pre-registered client for clients that ask for an id and secret.
- **Scopes:** `meetings.read` and `meetings.propose`. Access tokens last at most 1 hour and refresh tokens rotate. Tokens are never passed to another service.

**Workspace scope.**
- On consent, the user picks which workspaces this client may read, stored in `mcp_grants`.
- Each call computes the allowed set as the user's memberships intersected with the client grant, then applies the ADR 0004 rule before pagination.
- Tool arguments may narrow the workspace, never widen it.
- Personal end-to-end data cannot be reached: the server has no plaintext of it.

**Shared tool logic.** Move the tool catalog out of `mcp.rs` and `agent_tools.rs` into
`crates/meetings-tools` (MIT OR Apache-2.0). It holds names, JSON schemas, descriptions,
annotations, argument parsing, cursors, caps and the result envelope. It calls one trait:

```rust
pub trait ToolBackend {
    fn meetings(&self, f: &MeetingFilter) -> Result<Page<MeetingRow>>;
    fn meeting(&self, id: &str, parts: &Parts) -> Result<MeetingParts>;
    fn tasks(&self, f: &TaskFilter) -> Result<Page<TaskRow>>;
    fn memory(&self, scope: MemoryScope, q: &Query) -> Result<Page<Value>>;
    fn projects(&self) -> Result<Vec<Project>>;
    fn daily_check(&self, date: NaiveDate) -> Result<Option<String>>;
    fn propose(&self, p: Proposal) -> Result<ProposalReceipt>;
    fn features(&self) -> Features;
}
```

The `LocalBackend` reads the `Store` and `MeetingSource` (today's behavior over stdio). The
`ServerBackend` reads Postgres through the authorization module. A CI test checks that both
expose byte-identical tool schemas.

**Tools served remotely.**
- **The same 8 tools, same names.** Memory and daily check appear only when that workspace turns those features on.
- **`search` and `fetch`,** the pair ChatGPT expects for connectors and deep research. `search` returns ids, titles and URLs; `fetch` returns the full text of one result.
- **`propose_chat_summary`** (ADR 0007).
- **Every result carries** `workspace`, source provenance and the untrusted-data notice.
- **Writes only create Proposals.**
- **Limits:** 60 calls per minute per token, and an audit log without content.

**What each client needs** (verify in lane V2; both products change often):
- **ChatGPT:**
  - a public HTTPS URL;
  - OAuth with dynamic registration or metadata documents;
  - `search` and `fetch` for deep research;
  - developer mode for private use;
  - OpenAI app review before a public directory listing.
- **Claude.ai:**
  - a custom connector URL;
  - OAuth with dynamic registration, or a client id and secret in advanced settings;
  - the callback `https://claude.ai/api/mcp/auth_callback` allowed;
  - for Team and Enterprise plans, an owner who enables the connector.

## Options rejected

- An MCP TypeScript SDK sidecar: a second runtime for core logic.
- An MCP gateway (for example IBM Context Forge): another service, and authorization still has to be ours.
- Tunnelling the local stdio server: no multi-user auth, and the laptop must stay on.
- One generic `query` tool: the product review keeps the 8 named tools for clients that already use them.

## Consequences

- One tool catalog for local and remote use; a tool fix lands in both.
- The remote server needs a real domain, TLS and OAuth before any test with ChatGPT or Claude.ai.
- Cross-tenant tests become part of every server change.

## Revisit if

- The MCP authorization spec changes client registration again.
- OpenAI or Anthropic require a different tool shape or a review step we cannot meet.

## First step

Roadmap lane T1: extract `crates/meetings-tools` with `LocalBackend` and no behavior change.
CI compares a JSON snapshot of the local tool list before and after.
