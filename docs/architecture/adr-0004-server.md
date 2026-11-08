# ADR 0004: Server

Status: accepted, 2026-11-08. The auth provider choice is confirmed by the interop spike (lane V2).

## Context

Owner decisions B1 and D1: an AGPL, self-hostable server, about $3 per user per month, charged
only for sync and cloud. It has two jobs: a relay for encrypted personal sync (ADR 0003) and
server-readable team workspaces with a remote MCP endpoint (ADR 0005). The desktop app must
keep working with no server and no account.

## Decision

**One Rust service, `meetings-server`** (axum, tokio, sqlx, `object_store`). It has three surfaces:
- **Relay API:** puts and gets opaque encrypted batches and blobs, lists by cursor, and enforces quotas. It never parses payloads.
- **Workspace API:** entities from `meetings-domain`, search and ACL for team workspaces.
- **`/mcp`:** the remote MCP endpoint (ADR 0005).

**Storage.**
- **Postgres 16 or later** holds workspaces, memberships, entities (typed columns plus `jsonb`
  body), grants, proposals, the relay index, quotas, subscription state and the audit log.
- **Search:** Postgres full-text search plus `pg_trgm`; no extra search service.
- **Blobs:** go through the `object_store` crate, to any S3 API (AWS, Cloudflare R2, Garage) or to a local folder for a one-box install.

**Identity and auth.** The Rust server is an OAuth resource server only. It checks JWT access
tokens against the provider's JWKS (issuer, audience = this server's URL, expiry, scope).
- **Default provider:** Better Auth (MIT), run on Bun as its own container. It covers accounts, passkeys, email links, organizations if needed, and an OAuth or OIDC provider with dynamic client registration and PKCE for MCP clients.
- **Gate:** lane V2 must show that ChatGPT and Claude.ai finish OAuth and call a tool through it. If that fails, switch to Ory Hydra (Apache-2.0, OpenID certified, supports dynamic registration). The login and consent pages then move into the Rust service.
- **Swapping costs config, not code,** because the server only reads JWKS.
- **Self-hosters** may point app login at any OIDC provider (Keycloak, Authentik, Entra). Remote MCP additionally needs client registration support or a pre-registered client.
- **The desktop app signs in** with the loopback redirect and PKCE flow (RFC 8252).

**Workspace ACL tables.**

```sql
workspaces    (id uuid pk, name text, created_by uuid, created_at timestamptz)
memberships   (workspace_id uuid, user_id uuid,
               role text check (role in ('owner','admin','member','viewer')),
               primary key (workspace_id, user_id))
entities      (id uuid pk, workspace_id uuid, kind text, schema int, body jsonb,
               owner_id uuid, visibility text check (visibility in ('workspace','restricted')),
               copied_from jsonb, search tsvector,
               created_at timestamptz, updated_at timestamptz, deleted_at timestamptz)
entity_grants (entity_id uuid, user_id uuid,
               permission text check (permission in ('read','edit')), primary key (entity_id, user_id))
mcp_grants    (client_id text, user_id uuid, workspace_id uuid, granted_at timestamptz,
               primary key (client_id, user_id, workspace_id))
audit_log     (id bigserial, at timestamptz, actor uuid, client_id text,
               workspace_id uuid, action text, entity_id uuid)   -- never content
```

**One authorization rule, in one Rust module.** A user can read an entity when they are a member
of its workspace and the entity is `workspace`-visible, granted to them or owned by them. Every
list and search query applies this rule before `LIMIT`. A wrong or foreign id returns the same
"not found" as a missing one. Postgres row-level security on `entities`, keyed to a
per-transaction `app.user_id`, is the second line of defense.

**Self-host package.**
- `deploy/compose.yaml` starts `meetings-server`, `postgres`, `auth` (Better Auth) and `caddy` (TLS), with `garage` as an optional S3 store.
- `meetings-server` also ships as a single static binary.
- sqlx migrations are embedded. Settings come from environment variables; `.env.example` lists names only.
- Backup is `pg_dump` plus the blob folder, documented.
- Billing (Paddle) is a module that is off unless `BILLING=on`. A self-host without billing has no limits.

**AGPL boundary.** The repository becomes a Cargo workspace.

| Path | Licence |
| --- | --- |
| `server/` | AGPL-3.0-or-later |
| `crates/meetings-domain`, `crates/meetings-tools`, `crates/meetings-sync` | MIT OR Apache-2.0 |
| Desktop app | Owner choice, default MIT OR Apache-2.0 |

The server links the permissive crates, which is allowed. The app reaches the server only over
HTTP, so AGPL obligations never reach the app. Contributors sign off with DCO; no CLA. The
hosted service links its exact source commit at `/source` (AGPL section 13).

## Options rejected

- Supabase, PocketBase or Appwrite: several services or a fixed schema; more to carry than one Rust service.
- OpenFGA or SpiceDB now: another service; four roles and per-entity grants fit SQL.
- Zitadel: no dynamic client registration found for MCP clients (verify), and heavier to run.
- Keycloak: JVM with a large memory footprint for a small self-host.
- Cloudflare Workers: no self-host parity.
- A hand-written OAuth server inside the Rust service, with or without rmcp: security-critical protocol code written by hand.
- Tantivy or Meilisearch: another service before Postgres search is shown to be too weak.

## Consequences

- One language for server logic, sharing types and tool code with the app.
- Self-host runs 4 containers, including one Bun service for auth.
- ACL logic sits in one place and can be tested with a cross-tenant matrix.

## Revisit if

- Sharing needs folders or nested teams: move to OpenFGA.
- Search quality fails beyond about 1 million entities: add a Tantivy sidecar.
- Better Auth fails lane V2: switch to Ory Hydra.

## First step

Roadmap lane V1: server skeleton with health check, embedded migrations for the tables above,
JWT validation against a test JWKS, and `deploy/compose.yaml` started in hosted CI.
