Decisions needed: none.

# Meeting agent

## Purpose

Make Ask quick during a live meeting by defaulting to Codex with Low thinking and reusing the existing persistent app-server transport. The target is about three seconds to first answer text on a warm connection.

## Success criteria

- Models and agents has an Agent during meetings row using the installed agent and model catalog. Its thinking choices are Low and Medium.
- A live meeting uses that preference and shows it in Ask. Picker changes apply only to that meeting. Outside meetings retain their existing agent, model and thinking behavior.
- Meeting start initializes one Codex transport and primes it with one synthetic Ready request, without meeting text. Meeting end closes it after any current answer finishes. Two idle minutes release it too.
- Missing or failed Codex retries the same saved question with the normal Ask default and shows `Codex isn't available, using <name>.` Cancellation does not trigger a retry.
- All requested Rust gates, MCP smoke and old-settings reachability pass. Offscreen light and dark PNGs at 1280 show the row, its picker, live Codex Low and the fallback notice.
- At most six provider turns use synthetic meeting text. Record cold and warm Codex Low first-text and full-answer times, plus fallback evidence below.

## Constraints

Work only on feat/meeting-agent in this worktree. Use its own target folder with w64devkit first on PATH. Preserve summary behavior, existing setting keys, version and hooks. No push, packaging, install, desktop window or owner-screen input. Keep one warm transport and bound its idle lifetime. No real meeting data in measurements.

## Edge cases

- Codex missing or signed out: retry the same question using the normal Ask selection; if that also fails, restore the question for retry.
- Cold start: initialization and the synthetic Ready turn are asynchronous. A question submitted before warming finishes can exceed the warm target; never claim the target is guaranteed.
- Meeting ends during an answer: let the answer finish and save, then close its transport. Return subsequent Ask selections to normal defaults.
- Picker changes mid-meeting: keep a meeting-only agent/model/thinking override, reset the idle transport and retain typed text. An in-flight answer keeps its submitted selection.
- Next meeting: discard prior meeting overrides. A changed settings default does not silently rewrite the normal Ask selection.

## Out of scope

Summary agents, transcript generation, model installation or authentication, new providers, version changes, distribution and owner model configuration.

## Measurements and verification

The normal Ask selection remains separate from the live-meeting selection. Device preferences add `ask_meeting_agent` and `ask_meeting_model`, retaining `ask_meeting_thinking`. Old preferences default to Codex and Low. The new searchable row uses the existing installed-agent list and model catalog. The 49-key legacy reachability manifest is unchanged because its count describes old settings, not the current search index.

Ask snapshots the chosen agent, model, thinking and fallback when a question is submitted. A meeting-only picker change resets the idle transport and leaves typed text intact. Selection generations prevent an earlier fallback event from replacing a newer picker choice. A failed meeting question remains saved if another draft has already been typed.

Meeting start uses the existing persistent Codex stdio app server with read-only ephemeral threads and Low effort on turns. One synthetic Ready request warms the provider connection before the first real question. This adds one small provider request for a new warm meeting session and sends no meeting content. Already answered sessions are reused without another priming turn. Priming has a 30-second turn limit. One transport is retained. Two idle minutes release it; meeting end queues release after the current answer finishes. Outside-meeting transport behavior retains the existing ten-minute idle limit and does not add priming. The CLI JSON flags remain `--json` and `-c model_reasoning_effort=low` on the prepared Ask runner; the persistent transport uses app-server JSON messages and `effort: low`.

Fallback retries the original prompt once with the submitted normal Ask default and clears partial Codex text before displaying the replacement answer. Cancellation does not retry. Summary settings and their runners are unchanged.

The previous measurements are in ask-models.md and ask-models-review-1.md. The first probe confirmed that protocol initialization alone left the first question at 7.508 seconds to first text, while a reused connection reached 2.754 seconds. The implementation therefore adds the synthetic Ready turn. Final receipts follow below.


## Measured latency

Six provider turns in total, all synthetic. Measurements use the production live Ask path with an empty recorded-meeting context and an invented live transcript. The question asks what was decided and who will prepare it. The only facts are testing a paper prototype on Friday and Maya preparing it. Other worktrees were compiling on the shared laptop during measurement. Each row is one sample, not a latency guarantee.

| Call | First text, s | Full answer, s |
|---|---:|---:|
| Cold Codex Low, no prewarm | 17.230 | 19.454 |
| Reused Codex Low | 2.754 | 4.439 |
| Protocol-only prewarm, first question | 7.508 | 9.389 |
| Missing Codex, normal Claude Sonnet fallback | 26.984 | 29.219 |
| Final primed Codex Low, first meeting question | 2.240 | 3.750 |

Final warming took 14.415 seconds from initialization through the synthetic Ready result. The final question clock starts after readiness. A question submitted earlier waits for any remaining warm-up. This avoids paying that provider start on the first question once warming has finished. No meeting text is used for priming.

Call ledger: cold Codex question; reused Codex question; first question after protocol-only prewarm; Claude fallback question after a missing Codex executable; one synthetic Codex Ready turn; first question after Ready. Exactly six model requests, with no additional help or model-answer calls. Missing-executable resolution makes no provider request. The first four results are in meeting-agent-timings.json; the final two-turn proof is in meeting-agent-primed-timing.json. Both probes stopped their process trees cleanly.

## Final gates

All use this worktree's target directory with w64devkit first on PATH. Automated checks are rung 4 evidence.

| Command | Result | Seconds |
|---|---|---:|
| `cargo fmt --all --check` | PASS, exit 0 | 2.94 |
| `cargo clippy --workspace --locked --all-targets -- -D warnings` | PASS, exit 0 | 32.02 |
| `cargo clippy --workspace --locked --all-targets --features asr-runtime -- -D warnings` | PASS, exit 0 | 22.14 |
| `cargo test --workspace --locked` | PASS, exit 0 | 142.62 |
| `cargo test --workspace --locked --features asr-runtime` | PASS, exit 0 | 120.38 |
| `cargo build --locked` | PASS, exit 0 | 49.64 |
| `python tools/mcp-smoke.py E:\Personal-Projects\hex-meetings\.worktrees\meeting-agent\target\x86_64-pc-windows-gnu\debug\sidenote.exe` | PASS, exit 0 | 1.81 |
| `python tools/verify-meeting-agent.py E:\Personal-Projects\hex-meetings\.worktrees\meeting-agent\target\x86_64-pc-windows-gnu\debug\sidenote.exe` | PASS, exit 0 | 94.19 |

Each Rust test configuration passed 261 tests: 1 domain unit test, 14 domain integration tests, 196 app library tests and 50 UI tests. Isolated child tests are not counted twice. Both Clippy configurations denied warnings. MCP smoke verified nine tool schemas, JSON-RPC-only stdout, empty stderr and unchanged synthetic fixture files.

The final reachability check passed all 49 legacy keys across nine pages and the search reveal. The baseline manifest is byte-identical to release/dev15. One new searchable entry was added for the meeting-agent row; no legacy expected count was changed. The People editor and its Advanced fold are exercised to reach the existing team fields.

One initial full run failed an unchanged summary-settings fixture write with Windows AccessDenied, before the meeting assertions ran. The isolated rerun passed before any fixture change, and subsequent full parallel runs passed. The initial log is retained in artifacts/meeting-agent/gates/test-initial-failure.log. No hook, assertion, timeout or write guard was weakened. Renderer selector and fixture corrections are retained in the other failure logs.

## Offscreen proof

Rung 5 for the synthetic rendered flows and provider probes. All 38 PNGs are 1280 by 800 with matching warning-free element trees. They cover the readable installed-agent/model menu, Low and Medium only in meeting settings, live Codex Low, the fallback notice beside the preserved question, an actual model selection saved and read after restart, all nine Settings pages and the legacy controls. All eight requested images were also inspected visually. SHA-256 receipts are in artifacts/meeting-agent/verified-receipts.json.

Requested PNGs, relative to this worktree:

- `artifacts/meeting-agent/models-and-agents-1280-light.png`
- `artifacts/meeting-agent/models-and-agents-1280-dark.png`
- `artifacts/meeting-agent/meeting-agent-picker-1280-light.png`
- `artifacts/meeting-agent/meeting-agent-picker-1280-dark.png`
- `artifacts/meeting-agent/live-ask-codex-low-1280-light.png`
- `artifacts/meeting-agent/live-ask-codex-low-1280-dark.png`
- `artifacts/meeting-agent/fallback-notice-1280-light.png`
- `artifacts/meeting-agent/fallback-notice-1280-dark.png`

The signed-out failure policy, cancellation, meeting ending during an answer, selection changes, idle boundaries and retention of a newer draft are covered by automated tests. No account was signed out, no real recording was used, and no two-minute real-time idle recording was performed. No owner-screen input or visible app window was used.

The synthetic probe also exposed invalid live citation ids and missing citations. This remains outside the latency change and is recorded in https://github.com/harshanandak/hex-meetings/issues/3, re-fetched and verified open.

## Delivery

Implementation commits before this verification receipt:

- c374a9f docs: define meeting agent scope and verification
- 8edff50 feat(ask): add meeting fallback and bound warm session lifetime
- d78fa72 feat(ui): select Codex Low for live meeting questions
- b0677e0 fix(ui): keep meeting model choices readable and retain drafts
- 8b0ede2 feat(ask): prime meeting Codex and show fallback beside the input

Every commit has the requested Co-Authored-By trailer. Version 0.0.1-dev.15 is unchanged. Nothing was pushed, packaged or installed. The probe, renderer and final report are committed together in the verification receipt.

Elapsed: about 80 minutes including orientation, implementation, cold compilation, feature builds, UI corrections and final verification. Verification and rebuilds took most of the time. The largest avoidable rework was discovering picker clipping and notice placement after the first full gate pass.

## Round 1 accepted follow-up

Scope: remove thinking from the model picker in live and normal Ask. A question queued while Codex is still warming shows a muted `Starting Codexâ€¦` line under the box until non-empty answer text arrives. Warm-up progress must not trigger fallback. The existing error fallback remains unchanged.

Keep the same branch, isolated target, gates, version and screen restrictions. Add regression tests, re-render both live themes plus one warming state, and make no additional live provider calls. Final verification follows. No additional live provider calls were made; the six-call budget from the first round is unchanged.

The model field now contains only agent/model identity in both live and normal Ask. The live default displays its pinned `gpt-6.1-terra` model; normal Ask retains its existing `Codex` preset label. Low appears once, in the thinking picker.

Queued Codex warm-up work emits a starting progress event without changing the selected agent. The muted `Starting Codex?` line appears under a submitted question and survives initialization and empty text events. The first non-empty answer text clears it. Actual agent failures retain the existing fallback policy. Tests cover queue-state release, no fallback solely for warming, and the UI transition.

| Follow-up gate | Result | Seconds |
|---|---|---:|
| `cargo fmt --all --check` | PASS, exit 0 | 5.70 |
| `cargo clippy --workspace --locked --all-targets -- -D warnings` | PASS, exit 0 | 132.14 |
| `cargo clippy --workspace --locked --all-targets --features asr-runtime -- -D warnings` | PASS, exit 0 | 62.62 |
| `cargo test --workspace --locked` | PASS, exit 0 | 451.08 |
| `cargo test --workspace --locked --features asr-runtime` | PASS, exit 0 | 727.97 |
| `cargo build --locked` | PASS, exit 0 | 192.03 |
| `python tools/mcp-smoke.py E:\Personal-Projects\hex-meetings\.worktrees\meeting-agent\target\x86_64-pc-windows-gnu\debug\sidenote.exe` | PASS, exit 0 | 2.12 |
| `python artifacts/meeting-agent/round1-fixes/gates/verify-focused.py` | PASS, exit 0 | 81.66 |

Both test configurations passed 263 tests: 1 domain unit test, 14 domain integration tests, 198 app library tests and 50 UI tests. All 49 legacy expectations are unchanged. There were no Rust test, formatting, Clippy, build or MCP smoke failures this round. Automated checks are rung 4 evidence.

The offscreen run checked live and normal Ask in both themes, the warming line below the box, model preference persistence and the existing settings views. All 41 PNG/tree pairs match the final executable timestamp, have 1280 by 800 dimensions and no layout warnings. Requested views and both normal Ask views were inspected visually, rung 5 for those synthetic states.

A checker assumption that the live label must read Codex was corrected to use the configured model. The legacy search row was reachable, but its two-second highlight expired during a loaded capture. Its focused recapture and the unchanged reachability checker passed. One broader replay timed out during startup; that log is retained. No visibility/color assertion, application timeout or gate was weakened. Final receipts are in `artifacts/meeting-agent/round1-fixes`.

Updated requested PNGs:

- `artifacts/meeting-agent/live-ask-codex-low-1280-light.png`
- `artifacts/meeting-agent/live-ask-codex-low-1280-dark.png`
- `artifacts/meeting-agent/live-ask-warming-1280-light.png`

The same branch and isolated target were retained. No push, version change, packaging, installation or owner-screen input was performed.


## Round 2 final mechanism review

Purpose: keep one truthful progress status, prevent live panels from covering the page, use one composer width, default live Ask to the current meeting and show catalog model names.

Success criteria: Starting Codex appears once in the answer card until the actual turn starts; then the timer starts from that event. The composer uses the panel width at rest and with answers. The live document and transcript toggle stay above a bounded panel stack; the transcript scrolls and gives up space first. Live questions use only this meeting's transcript and typed notes unless the scope menu is widened. Outside defaults remain unchanged. Catalog names replace known ids and unknown ids stay intact.

Constraints and edge cases: preserve the branch, version, targets and gates. Keep drafts when scope changes and store live chats separately from all-meeting chats, including recordings without an id yet. Bound both scroll regions on narrow or short windows. Keep existing error fallback and cancellation. Use synthetic fixtures only, no extra provider requests and no owner-screen input. Finish this round and stop. Broader page redesign, citation work, model changes and distribution remain out of scope.

The answer card owns the warming spinner row. It shows `Starting Codex…` while a submitted question waits behind priming. The actual turn-start event clears that status and starts the thinking timer from zero. There is no separate status under the composer and no fallback caused by waiting.

The live page now lays out the title, notes and transcript toggle above the panel stack. Long notes have an internal scroll region. The stack uses the remaining pane height, with the transcript shrinking before the answer card; the transcript list shrinks and scrolls within its card. The live composer uses the panel width in both idle and answered states.

Live Ask defaults to This meeting. Its prompt includes the current transcript and typed notes, without scanning the saved meeting corpus. The scope menu can widen it to All meetings and return to the separate live conversation without losing a draft. Outside meetings, the existing scope and agent defaults remain. The model field resolves known catalog ids to names such as `GPT-6.1 Terra`; unknown ids are retained.

Regression coverage includes warming versus an actual turn, the timer reset, current-only prompt context, scope switching and draft retention, safe live conversation keys, panel height allocation, and known and unknown model names. The offscreen checker covers all three requested states in both themes at 1280 and 390 px, plus the scope menu and widened scope. No provider calls were added; the total remains six from the original implementation.

| Final round gate | Result | Seconds |
|---|---|---:|
| `cargo fmt --all --check` | PASS | 5.00 |
| `cargo clippy --workspace --locked --all-targets -- -D warnings` | PASS | 40.58 |
| `cargo clippy --workspace --locked --all-targets --features asr-runtime -- -D warnings` | PASS | 26.72 |
| `cargo test --workspace --locked` | PASS, 267 tests | 73.30 |
| `cargo test --workspace --locked --features asr-runtime` | PASS, 267 tests | 108.81 |
| `cargo build --locked` | PASS | 16.47 |
| `python tools/mcp-smoke.py target/x86_64-pc-windows-gnu/debug/sidenote.exe` | PASS | 1.48 |
| `python tools/verify-meeting-agent-round2.py target/x86_64-pc-windows-gnu/debug/sidenote.exe` | PASS, 14 frames | 45.11 |
| `python tools/verify-meeting-agent.py target/x86_64-pc-windows-gnu/debug/sidenote.exe` | PASS, 41 frames | 140.80 |
| `python tools/settings-reachability.py artifacts/meeting-agent` | PASS, 49 legacy keys | 0.39 |

Both test configurations passed 1 domain unit test, 14 domain integration tests, 199 app library tests and 53 UI tests. Isolated child tests are not counted twice. Both Clippy runs denied warnings. The legacy manifest and its expected count are unchanged. These checks are rung 4 evidence.

All 12 requested PNGs were inspected visually, rung 5 for these synthetic states. Each has a matching warning-free element tree and was captured after the final executable was built. The checks assert that the title, notes and transcript toggle finish above the panels; the inner transcript stays within its card; adding an Ask card shrinks the transcript; and the idle and answered composer widths match. The first narrow capture exposed an inner scroll area that did not shrink with its card. That was corrected before the final gate run. No timeout, assertion, hook or gate was bypassed.

Requested PNGs are in `artifacts/meeting-agent/round2/`:

| State | 1280 light | 1280 dark | 390 light | 390 dark |
|---|---|---|---|---|
| Idle | `live-ask-idle-1280-light.png` | `live-ask-idle-1280-dark.png` | `live-ask-idle-390-light.png` | `live-ask-idle-390-dark.png` |
| Warming | `live-ask-warming-1280-light.png` | `live-ask-warming-1280-dark.png` | `live-ask-warming-390-light.png` | `live-ask-warming-390-dark.png` |
| Answer, transcript open | `live-ask-answer-1280-light.png` | `live-ask-answer-1280-dark.png` | `live-ask-answer-390-light.png` | `live-ask-answer-390-dark.png` |

Gate logs and command receipts are in `artifacts/meeting-agent/round2/gates/`. PNG/tree hashes and the final executable hash are in `artifacts/meeting-agent/round2/verified-receipts.json`.

Elapsed: about 40 minutes. Reading and editing, verification, layout rework and waiting account for the work. Builds and verification took the largest share; the narrow transcript sizing was the main avoidable rework. The same branch and isolated target were retained. Version 0.0.1-dev.15 is unchanged. No push, packaging, installation, additional provider calls or owner-screen input was performed. This is the final mechanism round.
