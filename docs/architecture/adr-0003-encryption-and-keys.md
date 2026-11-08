# ADR 0003: Encryption and keys

Status: accepted, 2026-11-08. An independent cryptography review is a release gate for paid sync.

## Context

Owner decision A1: personal sync is end-to-end encrypted first; team workspaces come later
and are readable by the server. Today, sync is Syncthing between the owner's devices, with
plaintext at rest and BLAKE3 checksums that prove nothing about authorship (S02). The relay
in ADR 0004 must store personal data it cannot read, survive device loss, and let a user
remove a device.

## Decision

**Scope.** Only the relay path is end-to-end encrypted. `FolderTransport` (Syncthing) stays as
it is: device to device over Syncthing's TLS. Team workspaces are server-readable by design.

**Primitives.** No custom cryptography. The `age` crate (X25519 recipients,
ChaCha20-Poly1305 payloads) encrypts data and wraps keys. `ed25519-dalek` signs journals and
roster entries. Blob ids are BLAKE3 keyed with a per-space key, so the server cannot match
equal content across users.

**Keys.**

| Key | Where it lives | What it does |
| --- | --- | --- |
| Device keys (age identity + ed25519) | OS keystore through the `keyring` crate: Windows Credential Manager now, macOS Keychain later | Unwrap the space key; sign this device's journals |
| Space key, one per personal space per epoch (age identity) | Relay, only as copies wrapped to each device and to the recovery key | Every batch and blob is encrypted to it |
| Recovery key (age identity) | Shown once as text and a printable page; never uploaded | Restores access when every device is lost; can sign roster changes |

Keys never go in `config.json`, `device.json` or a synced folder. A test scans the data folder for key material.

**Device roster.** An append-only, signed log of `add` and `revoke` entries. Each entry holds
the device id, name, age recipient, ed25519 public key and epoch. The first device and the
recovery key sign the first entry; later entries need a valid device or the recovery key.
Clients check every batch signature against the roster and reject batches a revoked device
signed after its revocation.

**Pairing.** The new device shows a code and a QR holding its public keys. Both screens show
the same 6-digit check number. After the user confirms, the existing device signs an `add`
entry and wraps the space key to the new device. The relay only passes messages. The recovery
key can pair a device too.

**Revocation.** Any device, or the recovery key, appends `revoke`. That makes epoch n+1: a
new space key wrapped to the remaining devices and the recovery key, a compacted snapshot
re-encrypted under it, and the epoch-n blobs deleted from the relay. Limit, stated in the UI:
a removed device keeps whatever it already decrypted.

**Loss.** If one device remains, nothing is lost. If every device is lost, the recovery key
restores access. If the recovery key and every device are lost, the data is gone. Setup says so
before sync starts. A user can replace the recovery key from any device.

**Account versus data.** Relay login (ADR 0004) is separate. A password reset or account
recovery never grants data access.

**What the relay sees.** It sees the account, device count, batch sizes and times, and total
bytes. It cannot see store names, entity kinds, titles or text.

**Entering a team workspace.** Only an explicit "Copy to team" action does it:

- The app decrypts locally and sends plaintext over TLS to the workspace API. The copies are new entities with new ids and `copied_from {space, entity id, content hash, copied_at, copied_by}`.
- The dialog lists what goes. Summary, decisions and tasks are on by default; the transcript is off by default; chat threads are never included unless picked one by one.
- The dialog says: "Readable by the server operator and workspace members. Removing it later cannot recall copies others downloaded."
- A copied item shows "Team, server-readable". The personal original shows "Copied to <workspace> on <date>".
- Later personal edits never flow on their own. "Update team copy" repeats the explicit copy.

## Options rejected

- OpenMLS: group E2E protocol with large operating cost; A1 teams are server-readable.
- vodozemac: Matrix-specific, and not a recovery system.
- libsodium wrappers: more protocol written by hand.
- Passkey PRF as the only unlock: not available everywhere; add it later as a convenience.
- Server-held escrow keys "for recovery": breaks the E2E promise.
- Encrypting each batch to every device directly: a new device could not read history without re-encrypting everything.

## Consequences

- Only published, reviewed primitives are used; our own code is roster and wrapping logic, small enough to review.
- Revocation costs one re-encrypted snapshot per removal.
- Losing the recovery key and every device loses the data, by design.
- Search over personal data stays on devices; the server cannot index it.

## Revisit if

- Users ask for end-to-end encrypted teams. Then evaluate MLS for team spaces.
- Passkey PRF is available on every supported platform. Then add it as an unlock option.

## First step

Roadmap lane S5: a `KeyStore` trait with a `keyring` implementation and an in-memory test
implementation; create and load device keys. No sync changes yet.
