# Final preview security round

Decisions needed: none. This contract follows security review 2 at d67cd94.

## Threat model

1. These are the user's own devices; the Labs preview stays off by default.
2. Defend against a malicious relay operator and internet abuse of a public Worker.
3. Paired peers are trusted with all synced data.

## Scope and success criteria

- Confirmation timeout ends the entire invitation, clears its prompt and shows
  `Pairing timed out. Start again on both devices.` No attempt retry in that
  invitation. Each displayed confirmation and reply carries an attempt ID;
  stale replies cannot approve a different prompt or a later pairing job.
- Preserve `Receipt::Unknown` as an uncertain outcome. Persist and show the peer
  as `Pairing not confirmed`, with the owner's requested explanation. Do not
  turn a possibly delivered grant into a generic failure or build recovery/rekey.
- Pairing claim release checks a per-claim owner token. A second socket cannot
  release the first socket's IP claim. Empty `ALLOWED_ROOMS` rejects both public
  routes before any Durable Object allocation. Document the operator's edge
  rate-limiting rule; do not implement edge limiting in the Worker.
- Accept capability lockout by a malicious trusted peer, the parser's ordinary
  user token and unavailable removal for this preview. Document them and add the
  exact lost-device warning to the Sync preview note.
- Cover timeout, stale replies, uncertain grants, claim ownership and default
  denial with tests. Run the existing gates unchanged and render timeout,
  uncertain pairing and preview note in light and dark at 1280x800.

## Constraints and edge cases

Cancellation and timeout can race protocol completion. A terminal timeout must
not be overwritten by a failure event. New pairing jobs must reject replies from
older prompts. An unknown receipt can mean the guest already has the group key;
retain that state across restart. Existing claim rows without ownership expire
naturally. Health checks stay public and allocate no room.

No deployment, login, push, version bump or visible windows. Local tests use
synthetic data and numeric loopback, with explicit test-relay admission. Never
log invitation codes, group keys or claim tokens. Keep every gate and record any
failure. Leave the unrelated Memory undo failure and the accepted UI style alone.

Out of scope: capability recovery, restricted-token sandboxing, device removal,
distributed retirement, rekey, grant recovery, edge-limiter code and hosted work.
