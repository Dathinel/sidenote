# Roadmap: build order

Each lane is 1 to 3 days, runs in its own worktree, and is tested only in hosted CI. Foundation
lanes come first. Lanes with the same dependencies can run in parallel; keep 2 to 3 running at
once. The safety work in Wave 1 of `PLAN.md` (S03, S08, S09, S13, S04) runs alongside and is not
repeated here. UI lanes that show this work (Background tab, thread list, pairing screens) are
planned separately and start once their core lane lands.

Notation: **Deps** = lanes that must merge first. **Files** = the main files touched.
**Accept** = what CI or the reviewer must see before merge.

## Phase 0: foundation

**F0. Hosted CI** (1 day)
- **Deps:** the owner's remote choice (see the summary). **Files:** `.github/workflows/ci.yml`, `build.ps1` (find the GNU toolchain without a fixed path).
- **Accept:** each push runs `cargo fmt --check`, `clippy -D warnings`, `build --locked` and `cargo test` on `windows-latest`. The check is green on main and required for merges.

**F1. Domain crate** (2 days)
- **Deps:** none. **Files:** root `Cargo.toml` (workspace), `crates/meetings-domain/` (`ids.rs`, `entity.rs`, `provenance.rs`, `proposal.rs`, licence files).
- **Accept:**
  - `Id<T>` with v7 and v5 constructors.
  - The meeting id function matches golden vectors.
  - Every entity round-trips through serde.
  - No IO dependencies.

**F2. Store API v1 and JournalEngine** (3 days)
- **Deps:** F1. **Files:** `src/core/store/{mod,api,journal}.rs`, `sync_state.rs` moved under `store/`, `src/core/mod.rs`.
- **Accept:**
  - The `Store` trait and its adapter tests exist.
  - The journal format is unchanged (a golden file test).
  - Existing tests pass.

**F3. Ticks and chat messages on Store** (1 day)
- **Deps:** F2. **Files:** `src/core/persistence.rs`.
- **Accept:** it writes the same journal keys as before (golden test), and the UI is unchanged.

**F4. MeetingSource seam** (2 days)
- **Deps:** F1, F2. **Files:** `src/core/meetings.rs`, `src/core/agent_tools.rs`, `src/core/store/meetings.rs`.
- **Accept:**
  - The UI list and MCP use one catalog.
  - Each meeting has a UUID and its legacy id, and MCP accepts both.
  - The MCP output snapshot is unchanged apart from the new `uuid` field.

**F5. Tasks, memory and checks on Store** (3 days)
- **Deps:** F2. **Files:** `src/core/{tasks,memory,check_state,daily_check,config}.rs`.
- **Accept:** no file outside `src/core/store/` names `sync_state`, and a CI grep check enforces it.

**F6. Status files v1** (1 day)
- **Deps:** none. **Files:** `src/core/status.rs`, `src/listener.rs`.
- **Accept:**
  - The listener writes `local/status/listener.json` with a heartbeat.
  - Unit tests cover atomic writes and the 60-second stale rule.
  - This unblocks the Background UI lane.

## Phase 1: sync engine (ADR 0002, 0003)

**S1. Conformance suite and synthetic corpus** (3 days)
- **Deps:** F2. **Files:** `tests/sync_conformance.rs`, `tests/support/corpus.rs`, `docs/architecture/spikes/pilot-thresholds.json`.
- **Accept:**
  - Scenarios 1 to 12 and the property test run against `JournalEngine`.
  - The metrics JSON is uploaded as an artifact.
  - Scenarios 9 and 10 may be marked expected-fail until S2 lands.

**S2. Journal v3, part 1** (2 days)
- **Deps:** S1. **Files:** `src/core/store/journal.rs`.
- **Accept:**
  - Clock bound, change and size caps, and field-level keys for new kinds.
  - Scenarios 9 and 10 pass, and v2 is still read.

**S3. Automerge pilot** (3 days)
- **Deps:** S1. **Spec:** [spikes/automerge-pilot-spec.md](spikes/automerge-pilot-spec.md).
- **Accept:** `automerge-pilot-results.md` has M1 to M7 for both engines and a verdict, and ADR 0002's status is updated.

**S4. Own-journal compaction** (2 days)
- **Deps:** S2, and the S3 verdict keeping journals. **Files:** `src/core/store/journal.rs`.
- **Accept:** scenario 7 passes; compaction runs under the writer lock and writes backups first.

**S5. Keystore and device keys** (2 days)
- **Deps:** F1. **Files:** `crates/meetings-sync/src/keys.rs`, `src/core/keystore.rs`, `Cargo.toml` (`keyring`, `age`, `ed25519-dalek`).
- **Accept:**
  - Device keys are created and loaded from Windows Credential Manager.
  - CI uses the in-memory store.
  - A test finds no key material in the data folder.

**S6. Signed journals v3 and device roster** (3 days)
- **Deps:** S2, S5. **Files:** `src/core/store/journal.rs`, `crates/meetings-sync/src/roster.rs`.
- **Accept:**
  - Unsigned or forged journals after enrollment are rejected.
  - Roster add and revoke are verified.
  - The v2 acceptance rule from ADR 0002 holds.

## Phase 2: agents and pipeline (ADR 0006)

**H1. Harness trait and CliHarness** (3 days)
- **Deps:** none. **Files:** `src/core/harness/{mod,cli}.rs`, `src/core/{ask,ask_all,runner_process,runners}.rs`, `src/platform.rs`.
- **Accept:**
  - Ask and Ask All run through the trait.
  - Children start suspended and are attached to the job before resume (S11).
  - The child environment is cleaned (S05).
  - Provenance is returned.
  - The current policy is unchanged: OpenCode is still refused for Ask.

**H2. AcpHarness** (3 days)
- **Deps:** H1. **Files:** `src/core/harness/acp.rs`, `examples/fake_acp_agent.rs`, `Cargo.toml` (`agent-client-protocol`).
- **Accept:** CI tests against the fake agent cover:
  - session start and streaming;
  - a denied write permission;
  - a refused read outside the grant;
  - cancel and timeout.

**H3. Rust job runner with script stages** (3 days)
- **Deps:** F6. **Files:** `src/core/jobs/{mod,runner,stages}.rs`, `src/listener.rs`.
- **Accept:**
  - With `pipeline: "rust"` in config, the listener runs jobs in Rust in place of `hex-meeting-watch.ps1`.
  - Status is written per stage.
  - A job killed mid-stage resumes at that stage.
  - The 3-attempt retry rule holds.
  - CI uses fake scripts.

**H4. Quick title, notes and task classification on Harness** (3 days)
- **Deps:** H1, H3. **Files:** `src/core/jobs/stages/{quick_title,notes,tasks}.rs`, `assets/prompts/*.md.j2`.
- **Accept:**
  - A fake harness produces the expected files.
  - JSON outputs validate against schemas.
  - A CI grep finds no owner or company names in prompts.

**H5. Tool-free translation** (2 days)
- **Deps:** H4. **Files:** `src/core/jobs/stages/translate.rs`.
- **Accept:** `ToolPolicy::None` is enforced, a harness that cannot enforce it is refused, and S01 is closed.

**H6. Render and publish in Rust** (3 days)
- **Deps:** H3. **Files:** `src/core/jobs/stages/{render,publish}.rs`, `assets/templates/*.j2`.
- **Accept:**
  - Output matches the Python renderer on a synthetic meeting.
  - Publish is opt-in and backs up Hex originals.
  - Export paths stay inside configured roots (S10).

**H7. Audio preparation decision** (2 days, spike)
- **Deps:** H3. **Files:** `docs/architecture/spikes/audio-prep.md`.
- **Accept:** sherpa-onnx Rust bindings are compared with the uv Python path on synthetic audio in CI (time, memory, packaging size), and a recommendation is written.

**H8. Config v2: agents section** (2 days)
- **Deps:** H1. **Files:** `src/core/{config,runners}.rs`.
- **Accept:**
  - `runners.json` migrates with a backup first.
  - Migration is idempotent.
  - Uninstalled agents are marked "Not installed".

## Phase 2b: models and languages (ADR 0009)

Lanes marked **Owner audio** need the owner's real meetings to judge quality. Each of them runs once on the
owner's machine; the results file holds only numbers and labels, never transcript text. Every other lane is
judged in CI on synthetic or openly licensed clips.

Owner decisions of 2026-11-08 (see ADR 0009): everything builds on Hex; each language default is small
(ideally 100 to 300 MB) and fast on CPU, with bigger models opt-in; Sidenote uses a meeting model separate from
Hex's dictation model; small non-English models that Hex cannot load run through a provisional sherpa-onnx
runtime; users can declare their meeting languages and force a route; users may connect any OpenAI-compatible
endpoint, local or remote, with their own key. Every model size and quality figure stays unverified until M0
reports.

**M0. Real-meeting evaluation** (3 days; **public-clip results done 2026-11-09**, see ADR 0009 (b); real Hindi meeting audio still needed, first lane of Phase 2b)
- **Owner audio:** timings and error rates only.
- **Deps:** none. **Files:** `docs/architecture/spikes/pipeline-baseline.md`, `tools/pipeline_timings.ps1`. It also fills the `cpu_rtf`, `wer_hi_en` and `measured_on` fields for each tested model in the catalog.
- **Accept:**
  - Baseline: seconds per audio hour for prepare, each recognition pass, translation and notes, from the `audio-evidence.json` timing fields on at least 5 real meetings. Recognition calls per chunk are recorded.
  - Hex check: whether the local transcription service works on the owner's Windows build, and that `hex model transcribe --model <id>` leaves `transcription.model` unchanged.
  - Model comparison on consented real or mixed Hindi and English samples: Hex's installed Nemotron first, then the 150 MB Hinglish Whisper ONNX and a 190 MB IndicConformer pack through sherpa-onnx only if Nemotron fails on CPU speed or Hinglish quality. Each reports download size, CPU real-time factor and mixed-script WER (one normalization rule, written down).
  - Language ID is benchmarked separately from recognition, on mixed chunks.
  - The results file picks one default per language, or says none qualifies.
  - No transcript text leaves the machine, and no model is installed through `hex model install`.

**M1. Pipeline config v1** (2 days)
- **Deps:** H8. **Files:** `src/core/pipeline/{mod,config,provider}.rs`, `src/core/config.rs`.
- **Accept:**
  - The `pipeline` section, including `language_mode` (`auto`, `off` or `forced:<route>`) and the `endpoints` list (base URL and keystore key name, never the key), round-trips through serde (golden JSON).
  - Provider refs parse (`hex:`, `runtime:`, `endpoint:`, `acp:`, `cli:`, `off`).
  - A provider kind that a step does not allow is rejected with a plain message.
  - A config with no section loads as the `compat` profile, and migration is idempotent.

**M2. Model catalog file and search** (2 days)
- **Deps:** none. **Files:** `assets/catalog/models.toml`, `src/core/catalog.rs`, `docs/models/adding-a-model.md`, `.github/workflows/ci.yml` (catalog lint).
- **Accept:**
  - The CI lint checks the schema, SPDX ids, a `sha256` for each download URL and unique ids.
  - At least 15 seed entries, each with `source_url` and `verified_on`; size and quality fields read "unverified" until M0 measures them.
  - Unit tests cover search and the filters for task, language, size, licence and fit.

**M3. Hex model bridge** (2 days)
- **Deps:** M2. **Files:** `src/core/hex_models.rs`, `tests/support/fake_hex.rs`.
- **Accept:** against a fake `hex` in CI:
  - It parses `model list --json` v4 and `model devices`.
  - It never calls `hex model install`. A fake that records every call shows none, and `transcription.model` is unchanged after every operation.
  - GGUF import returns the new id.
  - One chunk is transcribed with `hex model transcribe --model <id> --language <code>` (or the local API with an explicit model), and the token never appears in logs.
  - A model that is not installed produces an "Install it in Hex" message, not an install.

**M4. Hardware tiers and fit badges** (1 day)
- **Deps:** M3. **Files:** `src/core/hardware.rs`.
- **Accept:** table tests map device lists to Light, Standard and GPU, and map catalog entries to "Fits GPU", "Runs on CPU, slower" or "Too large".

**M5. Language detection stage** (3 days)
- **Owner audio:** for the mixed-language accuracy check.
- **Deps:** H3, H7. **Files:** `src/core/jobs/stages/language_detect.rs`, `src/core/lid/{mod,anuvani}.rs`.
- **Accept:**
  - Each chunk gets a language, a confidence and the top two languages.
  - Detection is limited to the meeting languages, and the user can turn it off or force a route.
  - Labels are per-chunk guesses: the confidence and margin thresholds come from the mixed-speech benchmark, not from assumption.
  - A chunk under 2 seconds inherits the previous label.
  - Results are cached.
  - In CI: 95% or more correct on single-language clips of 3 seconds or more. Cost: Anuvani measured 12.2 s of one CPU thread per audio minute (about 730 s per audio hour); the earlier 60 s per hour target assumed Whisper tiny and was never measured. The lane measures multithreaded cost and keeps it under 15 s per audio minute.
  - On owner audio: a labelled sample of 100 chunks reports accuracy on mixed Hindi and English.
  - Detector decided 2026-11-09: Anuvani INT8 (ADR 0009 (b)). VoxLingua107 ECAPA is rejected (74% of Indian English sent to Hindi).

**M6. Routed recognition** (3 days)
- **Owner audio:** to choose the Hindi route.
- **Deps:** M1, M3, M5. **Files:** `src/core/jobs/stages/transcribe.rs`, `src/core/transcript_segments.rs`.
- **Accept:**
  - Each chunk gets one call, chosen by its route, or by the forced route (for example "Hindi + English mixed") when the user sets one.
  - English chunks reuse Hex's reading.
  - On the GPU tier only, a low-confidence chunk gets a second reading.
  - It writes `transcript.segments.v1.json` with the original text.
  - On a fake engine: 1.15 calls or fewer per chunk, and changing one route re-reads only those chunks.

**M7. Translate with the English skip** (2 days)
- **Deps:** H5, M6. **Files:** `src/core/jobs/stages/translate.rs`.
- **Accept:**
  - English chunks (confidence 0.8 or more) are never sent to translation.
  - An all-English meeting skips the step.
  - Translation is cached per chunk.
  - `text_en` and `text_original` are both kept.
  - The fake harness sees no English chunk.

**M8. EndpointHarness** (3 days)
- **Deps:** H1, S5 (keystore). **Files:** `src/core/harness/endpoint.rs`, `src/core/endpoints.rs`, `tests/support/fake_openai.rs`.
- **Accept:** against a fake server in CI:
  - It probes `/v1/models` and streams `/v1/chat/completions`.
  - It sends `response_format` with a JSON schema; on rejection it falls back to JSON mode, and the output is validated either way.
  - Any OpenAI-compatible URL works, local or remote, per pipeline step (owner decision O1). A remote URL must use https.
  - The API key is read from the OS keystore. A test finds the key in no config file, log line, journal or synced file.
  - Nothing is sent until the user picks an endpoint for a step, and new users start with local providers.
  - A one-time notice per endpoint names the host that receives meeting text, and audio when a speech step uses it. The send waits for the user's yes, and a changed host asks again.
  - ACP and CLI agents stay as they are; the generic endpoint covers every other provider.
  - Timeout and cancel work.

**M9. Local translator runtime** (3 days)
- **Owner audio:** the quality verdict. Owner decision O2 (the user's own agent translates Indian languages) is provisional and is revisited with the M0 results.
- **Deps:** M2, M7. **Files:** `src/core/translate/{mod,ctranslate2}.rs`, behind a `local-translate` cargo feature.
- **Accept:**
  - IndicTrans2 200M Indic to English runs as `runtime:indictrans2-indic-en-200m` after a user click installs it.
  - The hash is checked.
  - In CI: one sentence translates on the CPU runner.

**M10. Settings > Pipeline** (3 days)
- **Deps:** M1, M4. **Files:** `src/ui/settings_pipeline.rs`, a small entry in `src/ui/nav.rs`, `tools/render_ui.ps1` scenes.
- **Accept:**
  - Renders in dark and light at phone width.
  - Eight step rows, the language chips and "Use recommended".
  - The step sheet saves the provider, model and fallback.
  - The routes show for Speech.
  - The language handling choice (detect per chunk, off, forced route) saves to `language_mode`.
  - "Add endpoint" takes a URL and an API key (stored in the keystore) and shows the host notice before first use.

**M11. Settings > Models and "Try on a meeting"** (3 days)
- **Owner audio:** the comparison view replaces the scripts' side-by-side readings.
- **Deps:** M2, M3, M4, M6. **Files:** `src/ui/settings_models.rs`, `src/core/jobs/compare.rs`.
- **Accept:**
  - Search and filter chips work.
  - The licence dialog appears before any install, and a test shows that opening the screen makes no network call.
  - "Add your own" covers GGUF, URL, ACP and CLI.
  - "Try on a meeting" runs two providers on 10 chunks of a chosen meeting and shows them side by side, cached.

**M12. sherpa-onnx recognition runtime, provisional** (3 days)
- **Deps:** M0 (a model that M0 selects but Hex cannot load), M2, H7. **Files:** `src/core/asr/{mod,sherpa}.rs`, `assets/catalog/models.toml`, packaging notes in `docs/architecture/spikes/`.
- **Accept:**
  - `runtime:<id>` speech providers run tested ONNX packs through sherpa-onnx, with each file's `sha256` checked and a download only after a user click.
  - The first pack is the one M0 picks (for example the 150 MB Hinglish Whisper or a 190 MB IndicConformer pack).
  - In CI, one clip transcribes on the CPU runner, and the Windows package size and memory are recorded.
  - The code sits behind a cargo feature and one trait, so a later lane can remove it without touching the router.

**M13. Upstream Hex contribution** (3 days; owner approval before any pull request)
- **Deps:** M0. **Files:** a fork of Hex (not this repo), plus a short design note in `docs/architecture/spikes/hex-upstream.md`.
- **Accept:**
  - Support for [issue #1351](https://github.com/anomalyco/hex/issues/1351): a separate meeting-model setting, reviewed against `src/app_settings/meetings.rs`, `src/meeting/runtime.rs` and the settings UI.
  - An install command that downloads a model without selecting it.
  - Hex loads at least one compact model that M12 runs today, or the note states why it cannot.
  - When these land in a Hex release, a follow-up lane removes M12's runtime for the models Hex now loads, with a migration that keeps the user's routes.

Order: M0 is running now and gates every model choice. M2 and M8 can start now (M8 also needs S5). M1 waits
for H8. M5 waits for H3 and H7. M12 and M13 wait for the M0 results. The UI lanes M10 and M11 come last.

## Phase 3: chat history (ADR 0007)

**C1. Thread model and migration** (2 days)
- **Deps:** F3. **Files:** `crates/meetings-domain/src/chat.rs`, `src/core/chat.rs`.
- **Accept:** legacy messages become threads with deterministic ids, identical on two simulated devices, and migration is idempotent.

**C2. Provenance capture** (1 day)
- **Deps:** H1, C1. **Accept:** a reported model is stored as reported, otherwise as requested, otherwise unknown.

**C3. Export and import** (1 day)
- **Deps:** C1. **Accept:** JSON and Markdown export per thread and for all threads, `--export-chats`, and a JSON round trip.

**C4. Local FTS5 index** (3 days)
- **Deps:** C1, F4. **Files:** `src/core/index.rs`, `Cargo.toml` (`rusqlite`, bundled).
- **Accept:**
  - The index rebuilds from the Store.
  - Search covers meetings, tasks and chat.
  - Deleting `index.sqlite` loses nothing.

**C5. `propose_chat_summary`, local** (2 days)
- **Deps:** C1. **Files:** `src/core/agent_tools.rs`, `src/mcp.rs`, `src/core/config.rs`.
- **Accept:**
  - The tool creates a pending proposal and deduplicates by hash.
  - It is gated by `chat_summaries`.
  - Accepting it creates a thread with provenance.

## Phase 4: server, sync relay and remote MCP (ADR 0003, 0004, 0005)

**T1. `meetings-tools` crate** (3 days)
- **Deps:** F4, F5. **Files:** `crates/meetings-tools/`, `src/mcp.rs`, `src/core/agent_tools.rs`.
- **Accept:** the local tool list JSON snapshot is identical before and after the change, through `LocalBackend`.

**V1. Server skeleton** (3 days)
- **Deps:** F1. **Files:** `server/` (AGPL licence, `main.rs`, `config.rs`, `migrations/0001_init.sql`), `deploy/{compose.yaml,Caddyfile,.env.example}`.
- **Accept:**
  - Compose starts on an Ubuntu CI runner.
  - Health, migrations and JWT checks against a test JWKS all pass.

**V2. Auth interop spike** (2 days)
- **Deps:** V1, plus a staging domain and test accounts from the owner. **Files:** `docs/architecture/spikes/auth-interop.md`.
- **Accept:**
  - Claude.ai and ChatGPT both finish OAuth through Better Auth and call a stub `search`.
  - The results are recorded.
  - ADR 0004 is confirmed, or switched to Ory Hydra.

**V3. Relay API** (3 days)
- **Deps:** V1. **Files:** `server/src/relay.rs`.
- **Accept:**
  - Put and get by opaque id, list by cursor, idempotent puts and per-account quota.
  - The server never parses payloads.

**V4. Encrypted relay transport** (3 days)
- **Deps:** S6, V3. **Files:** `crates/meetings-sync/src/{envelope,relay}.rs`, `src/core/store/transport.rs`.
- **Accept:**
  - Two simulated devices sync through the relay in CI.
  - A test searches server storage for corpus strings and finds none.

**V5. Pairing and recovery** (3 days)
- **Deps:** V4. **Accept:** pairing with the 6-digit check number works, and a fresh device restores from the recovery key in CI.

**V6. Revocation and epoch rotation** (2 days)
- **Deps:** V5. **Accept:**
  - After revoke, the removed device cannot read new batches.
  - Old-epoch blobs are deleted from the relay.

**V7. Workspace ACL and authorization** (3 days)
- **Deps:** V1. **Files:** `server/src/authz.rs`, `migrations/0002_acl.sql`.
- **Accept:**
  - In the cross-tenant matrix, guessed ids, pagination and search all deny.
  - Row-level security blocks a query that skips the module.

**V8. Copy to team** (3 days)
- **Deps:** V7, F4, C1. **Files:** `server/src/workspace.rs`, `src/core/team.rs`.
- **Accept:**
  - The dialog content matches ADR 0003.
  - The transcript is off by default.
  - `copied_from` is set and both labels show.

**V9. Remote MCP** (3 days)
- **Deps:** T1, V7, V2. **Files:** `server/src/mcp.rs`.
- **Accept:**
  - `/mcp` answers 401 with resource metadata and checks the token audience.
  - Results are scoped to the workspace.
  - `search` and `fetch` work.
  - Schemas match the local snapshot.

**V10. Billing** (3 days)
- **Deps:** V3, plus a cost check against the $3 price (owner decision D1) and Paddle eligibility for India. **Files:** `server/src/billing.rs`.
- **Accept:** with `BILLING=off`, there are no limits. Webhooks update quota.

**Gate before paid launch:**
- independent cryptography and security review of V4 to V9;
- privacy statement and SECURITY.md.

## Phase 5: distribution (ADR 0008)

**D1. Velopack release** (2 days)
- **Deps:** F0. **Files:** `src/main.rs`, `.github/workflows/release.yml`, `Cargo.toml` (`velopack`).
- **Accept:**
  - A tag produces `Setup.exe` and delta packages.
  - CI installs N and updates to N+1 on a clean runner.

**D2. Supply chain** (1 day)
- **Deps:** F0. **Files:** `deny.toml`, `.github/workflows/ci.yml`.
- **Accept:** `cargo-deny` checks licences and advisories, an SBOM is uploaded, and attestations are on.

**D3. Signing** (1 day, plus approval time)
- **Deps:** D1, the privacy gate (S04) and a public repository.
- **Accept:** a signed `Setup.exe` and exe, with the publisher shown in Windows.

**D4. macOS discovery** (2 days, spike, later)
- **Deps:** D1. **Accept:** a written check of Rune, Hex and the platform adapters on macOS, with a port estimate.

## Suggested first three parallel lanes

1. F0 + F1 (CI and the domain crate). F0 is short, so one builder can do both.
2. F6 status files (unblocks the Background UI).
3. H1 harness trait (closes S05 and S11 early).

Then F2, F3 and F4, then S1 and C1 in parallel.
