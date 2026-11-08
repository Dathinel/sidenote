# meetings-domain

Shared Rust types for the desktop app and server, under MIT OR Apache-2.0.
This crate implements the F1 contracts in ADR 0001, 0002, 0006 and 0007.

- `Id<T>` keeps entity kinds distinct in Rust and serializes as a UUID string.
  New ids use v7. Imports use v5 with a fixed namespace per entity kind.
- Meeting ids use the exact UTF-8 key `hex/<legacy device id>/<Hex folder>`.
  The helper rejects empty device keys, `/` in device keys and invalid Hex folders.
  Namespaces are v5 of the URL namespace and `https://hex-meetings.org/ns/<kind>`.
  This URL is a permanent name, not a network endpoint. Do not change these constants.
- All 14 entity payloads go inside `Envelope<E>`. The envelope holds the typed id,
  workspace, schema, UTC timestamps, creator, tombstone and optional provenance.
  The payload is under `data`, separate from metadata and engine version tokens.
- Chat provenance lives on the envelope, including harness and model observations,
  the reported/requested/unknown choice, tools, task and context limits.
- Transcripts and notes point to content blobs. Chat text enforces the 32 KiB inline
  limit on construction and deserialization. Callers write larger text as blobs.
- `Entity::upgrade` runs before payload deserialization. Schema 1 is the first
  format; schema 0 has no migration. Future changes override the pure upgrade hook
  and add tests. Reads normalize to the latest schema, which is private on envelopes.
  Newer schemas return "Update Meetings on this device" before decoding the payload.
- `Version` is an opaque token owned by a Store engine. Proposal constructors create
  pending entries. Store implementations enforce review authorization and completed
  chat message immutability. This crate performs no storage or agent execution.

Runtime dependencies are serde, serde_json, chrono (without its clock feature) and
uuid. UUIDv7 uses OS randomness and the system clock. There are no filesystem,
network, database, async runtime or UI APIs. Device records hold roster metadata;
device configuration, credentials, indexes and jobs belong outside this crate.
Pipeline steps and provider/model directory types belong to ADR 0009's lane.

Tests use synthetic data only. Golden UUID vectors were generated with Python's
standard-library UUIDv5 implementation. CI runs unit and property tests; local
`cargo clippy --locked --all-targets -- -D warnings` compiles them without execution.
