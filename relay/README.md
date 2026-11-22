# Sidenote relay

Decisions needed: none for slice 1. Copied from synccore-r1 (461afde). The app integration contract and release gates are in [sync-relay-integration.md](../docs/internal/sync-relay-integration.md). No deployment or account login is performed by this slice.

## Self-hosting

Start with the [Sidenote self-host guide](../docs/self-host-relay.md). It covers
prerequisites, the pinned local commands, configuration, Settings > Sync >
Advanced, removal/rotation and costs. Every command in that guide was run locally;
account provisioning and publication are clearly marked as unexecuted steps.
No deployment, authentication or billing change was performed for this slice.

## Admission and residual risk

Before any Durable Object lookup, the Worker rejects pairing room names outside the 1,000 precomputed SHA-256 locator hashes. Sync logins require valid signed membership and are limited to ten successful device-key admissions per minute across rooms/locations by one short-lived Durable Object per public key. Invalid signatures cannot spend another device's login budget. Per-key counters are deleted by alarm after their minute window.

Pair rooms permit at most four unauthenticated sockets, 64 authenticated client frames, three owner retry controls, and three guest attempt reservations. A new socket evicts the oldest unauthenticated socket when that cap is reached. Unauthenticated frames do not spend the room budget; 64 frames cover three complete pairing attempts plus margin. Limits persist through hibernation. The owner remains connected if a frame has no guest recipient. Auth expires after 30 seconds; invitations expire after ten minutes. Grant consumes the invitation durably before the bundle is forwarded once; completion or a 30-second grant grace deadline schedules cleanup. Expired/consumed room SQL is deleted by alarm. Sync pins are rewritten only on roster expansion.

Frame size is 128 KiB, checked on strings before UTF-8 encoding. Each socket is capped at 600 messages and 16 MiB per one-second window. There are at most 32 sockets per room and 16 members per group. Enrollment member public keys are validated before their proofs are accepted. These are work/admission bounds, not strict global IP protection.

Cloudflare's [Workers Rate Limiting binding documentation](https://developers.cloudflare.com/workers/runtime-apis/bindings/rate-limit/) and [GA announcement](https://developers.cloudflare.com/changelog/post/2025-09-19-ratelimit-workers-ga/) describe the API and approximate per-location counters, but do not explicitly establish Workers Free entitlement. The documentation's application-user free/paid example is not Cloudflare plan availability. **Free entitlement could not be verified; no IP Rate Limiting binding is provisioned.** This conditional part of R1 remains unverified rather than being presented as available or definitively unavailable. The device-key limiter uses the existing free-compatible SQLite DO namespace.

Every request reaching the Worker can still consume the Free allowance, even when the Worker rejects it before a DO call. A determined stranger can exhaust Worker/DO quotas and interrupt sync. Public locators also permit burning invitation attempts. The owner accepts this residual risk and can use a self-deployed relay. There is no custom-domain/WAF mitigation in this project.

If limits are hit, move the account to **Workers Paid, starting at $5/month**. [Workers pricing](https://developers.cloudflare.com/workers/platform/pricing/) includes ten million requests and thirty million CPU milliseconds monthly; excess requests cost $0.30/million and CPU $0.02/million ms. [Durable Objects pricing](https://developers.cloudflare.com/durable-objects/platform/pricing/) includes one million DO requests and 400,000 GB-seconds monthly, with excess usage billed separately. SQLite includes 25 billion rows read, 50 million rows written and 5 GB-month storage. Free DO limits are 100,000 requests and 13,000 GB-seconds daily. Paid increases allowances; $5 is a minimum, not an unlimited flat rate. The upgrade is an owner action, not a code deployment or a guarantee against denial of service.

## Storage and privacy

Production `src/index.ts` uses WebSocket Hibernation handlers, attachments and automatic relay ping/pong. Observability/metrics are disabled. It has no payload logging or message storage. SQL stores public pins plus bounded non-content pairing/login counters and deadlines; alarms delete transient metadata. Ciphertext exists only in runtime/network buffers while forwarded. Auth nonces and socket routing/rate state reside in hibernation attachments. No password, group key, PAKE state, note body, ciphertext backlog or offline message is written.

The relay can observe IPs, public membership, room/session IDs, sizes and timing. It can censor or drop data. Pinned device signatures and encrypted Noise sessions protect content; relay ping responses alone do not make a client report synced. Clients need another online device for catch-up.

## Round-one gates and CI-only execution

Run `bun test tests` and, with the pinned dependencies installed, `bun run typecheck` from this folder. Run Rust workspace gates from the repository root. The app integration test starts installed Wrangler locally on numeric loopback with disposable state. It does not deploy or log in.

`.github/workflows/ci.yml` installs relay dependencies with Bun and runs the workspace tests, including two headless app processes with local Wrangler. Build the debug helper and examples before running the tests directly. Product pairing is exercised by the isolated app tests. Physical-device testing, a hosted relay and independent security review remain release gates.

The CI harness command is `python harness/run.py` from `relay/`; `--quick` reduces measured edits/idle time. It starts only numeric-loopback Wrangler on ports 8787/9237 with fresh disposable state and TEMP client databases. The harness terminates its own process tree and opens no windows or browser. This is local emulation, not cloud deployment.

`harness/observed.ts` is a separate CI entry point. It captures incoming/outgoing auth/control, pair binaries, every signed envelope kind and SQLite schema/cell bytes in ignored `.local-evidence/ciphertext.log`. It disables automatic ping response only in that observer so those controls are captured. A local proof route plants the actual marker in SQL; the scanner must find it in the tagged negative control and nowhere else. Scanner unit tests inject the marker into each surface and require rejection, and require all capture kinds. Production hooks are no-ops and expose no proof routes. Raw evidence must remain disposable; the report contains only aggregates.

Historical evidence from the source branch was not copied. The standalone core harness remains available for additional qualification; its results are separate from the app integration test. See the integration plan for the threat model, migration boundary and work that remains before release.
