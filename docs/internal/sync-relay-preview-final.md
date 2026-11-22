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

## Accepted limits

For this default-off, own-device Labs preview, capability lockout by a malicious
paired peer, the parser's normal user token, and unavailable Remove are accepted
limitations. No capability recovery, additional sandbox, removal or rekeying
mechanism is added. A paired peer is already trusted with all shared data.

## Verification receipt, 2026-11-11

Local execution evidence, not an independent security verdict. Code under test:
`a42b2d0` and `ed0e90b`. All 18 serialized verification commands exited zero.
Commands, durations and complete logs are in
`artifacts/relay-final-round/results.json`. GNU tools were first on PATH, builds
used this worktree's own target, and no hosted service or visible window was used.

| Gate | Result |
| --- | --- |
| `cargo fmt --all --check` | Passed |
| `cargo clippy --workspace --locked --all-targets -- -D warnings` | Passed |
| Same Clippy command with `--features asr-runtime` before `--` | Passed |
| `cargo build --workspace --locked --bins --examples` | Passed; headless fixtures |
| `cargo test --workspace --locked` | Passed |
| Same test command with `--features asr-runtime` | Passed |
| `cargo build --locked` | Passed |
| `python tools/mcp-smoke.py <this worktree>/target/x86_64-pc-windows-gnu/debug/sidenote.exe` | Passed |
| `bun test tests` in `relay` | 34 passed, 0 failed |
| `bun run typecheck` in `relay` | Passed |
| Settings renders, team editor fixture, reachability and review checks | Passed; 101 PNGs, all 49 legacy keys reachable |
| Relay renders and all three relay render checkers | Passed; 32 PNGs |

Both workspace runs passed 188 application library tests, 47 UI tests, all five
real isolated-app/loopback Worker cases and 53 sync-core tests, plus workspace
helper and documentation tests. The previously failing Memory undo test passed
in these runs; its code and test were not changed.

New regressions exercised:

- `expired_confirmation_consumes_invitation_without_a_retry_or_grant`
- `confirmation_timeout_ends_job_and_expired_replies_cannot_approve_next_prompt`
- `a_reply_for_another_attempt_never_confirms_the_current_prompt`
- `uncertain_pairing_status_and_peer_name_survive_restart`
- Worker: `closing a second unauthenticated owner socket cannot release the first IP claim`
- Worker: `empty room allowlist denies owners guests and sync before any object allocation`
- Worker: `public owner requests replace any caller supplied claim token`

The existing C6 fault test also passed its confirmed, unsent and unknown receipt
cases. `tools/check-relay-final-renders.py` passed six 1280x800 frames in
`artifacts/relay-final-round/relay-ui`: `relay-timeout-{light,dark}.png`,
`relay-unconfirmed-{light,dark}.png` and `relay-preview-{light,dark}.png`.
All six were visually inspected. The timeout has no old confirmation controls;
the uncertain outcome and lost-device note are visible in both themes.
`artifacts/relay-final-round/evidence.json` records the PNG hashes and gate results.

No recovery, rekeying, edge-limiter code or changes to the accepted limitations
were added. The Cloudflare zone, route and rate-rule instructions still require
the account owner and have not been exercised against a hosted service.
