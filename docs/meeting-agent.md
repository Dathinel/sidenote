Decisions needed: none.

# Meeting agent

## Purpose

Make Ask quick during a live meeting by defaulting to Codex with Low thinking and reusing the existing persistent app-server transport. The target is about three seconds to first answer text on a warm connection.

## Success criteria

- Models and agents has an Agent during meetings row using the installed agent and model catalog. Its thinking choices are Low and Medium.
- A live meeting uses that preference and shows it in Ask. Picker changes apply only to that meeting. Outside meetings retain their existing agent, model and thinking behavior.
- Meeting start initializes one Codex transport without sending meeting text or a hidden model request. Meeting end closes it after any current answer finishes. Two idle minutes release it too.
- Missing or failed Codex retries the same saved question with the normal Ask default and shows `Codex isn't available, using <name>.` Cancellation does not trigger a retry.
- All requested Rust gates, MCP smoke and old-settings reachability pass. Offscreen light and dark PNGs at 1280 show the row, its picker, live Codex Low and the fallback notice.
- At most six provider turns use synthetic meeting text. Record cold and warm Codex Low first-text and full-answer times, plus fallback evidence below.

## Constraints

Work only on feat/meeting-agent in this worktree. Use its own target folder with w64devkit first on PATH. Preserve summary behavior, existing setting keys, version and hooks. No push, packaging, install, desktop window or owner-screen input. Keep one warm transport and bound its idle lifetime. No real meeting data in measurements.

## Edge cases

- Codex missing or signed out: retry the same question using the normal Ask selection; if that also fails, restore the question for retry.
- Cold start: initialization is asynchronous. The first provider turn can exceed the warm target; never claim the target is guaranteed.
- Meeting ends during an answer: let the answer finish and save, then close its transport. Return subsequent Ask selections to normal defaults.
- Picker changes mid-meeting: keep a meeting-only agent/model/thinking override, reset the idle transport and retain typed text. An in-flight answer keeps its submitted selection.
- Next meeting: discard prior meeting overrides. A changed settings default does not silently rewrite the normal Ask selection.

## Out of scope

Summary agents, transcript generation, model installation or authentication, new providers, version changes, distribution and owner model configuration.

## Measurements and verification

Pending implementation. The previous measurements are in ask-models.md and ask-models-review-1.md. Initializing Codex alone does not prime its first provider turn.
