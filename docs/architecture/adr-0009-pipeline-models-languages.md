# ADR 0009: Configurable pipeline, language routing and model directory

Status: proposed, 2026-11-08. Amends ADR 0006 (adds a provider per stage and an `EndpointHarness`).

## Context

- **Today's scripts read every chunk several times.** `meeting-audio.py` splits each track into speech chunks of 24 seconds or less (sherpa-onnx VAD and diarization), then sends every chunk to two language-specific Hex models through the Hex local API (`POST /transcriptions?model=&language=`). An LLM then merges those two readings with Hex's own English reading and translates in batches of about 18,000 characters. A one-language meeting pays for three readings and a full LLM pass.
- **Models and agents are fixed in the scripts:** an OpenCode model for translation, Claude Code (Sonnet) for notes, Claude Code (Haiku) for the quick title and a Python call for tasks.
- **What Hex offers** (read from `hex model list --json` v4, `model devices` and `settings list`):
  - A speech catalog of 17 entries. Each has `languages`, `size`, `installed`, `streams`, `vocabulary` (bias, prompt or none) and `source` (hex, download or imported).
  - `model import` copies a GGUF file into Hex, and `model transcribe --model --language` transcribes a WAV file.
  - `model devices` lists the compute devices, for example a Vulkan GPU with 6 GB and the CPU with its RAM.
- **Hex's limits:**
  - Meetings and dictation share one model: there is only `transcription.model` and `transcription.language`, with no meeting-specific key.
  - Hex does not detect the spoken language. The language is fixed per model choice; a Labs option switches it by keyboard layout, and Whisper-family models accept `auto`.
  - `model install` downloads a model **and selects it**, which would change the user's dictation model.

## Decision

### (a) The pipeline is eight configurable steps

| Step | ADR 0006 stage | Allowed provider kinds | Default |
| --- | --- | --- | --- |
| capture | detect | `hex` | Hex meeting recorder |
| language detect | language_detect (new, after prepare_audio) | `runtime`, `hex` (Whisper `auto`), `off` | `runtime:whisper-lid-base` |
| speech | transcribe | `hex`, `runtime`, `endpoint` (`/v1/audio/transcriptions`) | routed by language (see b) |
| translate | translate | `runtime`, `endpoint`, `acp`, `cli`, `off` | the user's agent, `ToolPolicy::None` |
| quick title | quick_title | `endpoint`, `acp`, `cli` | the agent's smallest model |
| notes | notes | `endpoint`, `acp`, `cli` | the user's agent |
| tasks | classify_tasks | `endpoint`, `acp`, `cli` | the agent's smallest model |
| memory | memory (new, last) | `endpoint`, `acp`, `cli`, `off` | `off`; writes Proposals only (ADR 0001, rule 5) |

**Provider kinds.** A provider is a string with a kind prefix:

| Kind | Example | Runs through |
| --- | --- | --- |
| Hex model | `hex:<model-id>` | Hex local API or CLI |
| Local model runtime | `runtime:<catalog-id>` | ONNX or CTranslate2 inside the job runner |
| Local LLM endpoint | `endpoint:<name>` | OpenAI-compatible HTTP |
| ACP agent | `acp:<agent-id>` | ADR 0006 `AcpHarness` |
| CLI agent | `cli:<agent-id>` | ADR 0006 `CliHarness` |

`hex:meeting-transcript` means Hex's own meeting transcript, used as is.

**LLM steps use the `Harness` trait.** Translate, quick title, notes, tasks and memory all go through ADR 0006's `Harness`.
- A third implementation, `EndpointHarness`, calls `/v1/chat/completions` and probes `/v1/models`.
- Ollama, LM Studio and llama.cpp `llama-server` all expose these endpoints.
- It offers no tools, so `ToolPolicy::None` always holds.
- For JSON outputs it sends `response_format` with a JSON schema. When the server rejects that, it falls back to JSON mode, and validates the result either way.

**Audio steps use two small traits.** `LanguageId::identify(chunk) -> (lang, confidence, top2)` and `SpeechEngine::transcribe(chunk, lang) -> (text, confidence)`.

**Fallbacks.** Every step has one `fallback` provider. When the chosen provider fails probe or run, the job tries the fallback once, then follows ADR 0006's retry rule.

**Config lives in device-local `config.json`** (ADR 0001 class 4; it never syncs):

```json
"pipeline": { "schema": 1, "profile": "recommended", "meeting_languages": ["hi", "en"],
  "steps": {
    "language_detect": { "provider": "runtime:whisper-lid-base", "fallback": "off" },
    "speech": { "routes": { "en": "hex:parakeet_unified_en", "hi": "hex:nemotron_3_5_asr",
                            "*": "hex:whisper_large_v3_turbo" }, "fallback": "hex:meeting-transcript" },
    "translate": { "provider": "acp:claude-code", "model": null, "fallback": "endpoint:ollama", "skip_english": true },
    "notes": { "provider": "acp:claude-code", "fallback": "cli:codex" } } }
```

**Hardware tiers.** Tiers come from `hex model devices`; "Use recommended" applies the tier's defaults.

| Tier | Hardware | Default behaviour |
| --- | --- | --- |
| Light | CPU only, under 16 GB RAM | No language detection; Hex's meeting transcript is used as is; agents do the text steps. |
| Standard | CPU with 16 GB RAM or more, or a GPU under 6 GB | Whisper tiny language detection; one routed recognition pass. |
| GPU | 6 GB VRAM or more | Whisper base language detection; a second reading for low-confidence chunks; local endpoints offered. |

### (b) Language handling

1. **Declare the meeting's languages.** The user lists the languages their meetings use, for example Hindi and English. Detection is restricted to that list, which turns a 99-way choice into a 2-way choice and cuts errors.
2. **Detect per chunk** on the existing VAD chunks, which are 24 seconds or less.
   - **Default detector:** Whisper language identification through sherpa-onnx. It ships tiny, base, small and medium models (no large). The tiny model ran at real-time factor 0.043 in sherpa's own example.
   - **Why per chunk:** `whisper.transcribe()` detects once, from the first 30 seconds, then fixes the language for the whole file. That fails on code-switched meetings, so a per-chunk call is required.
   - **Short chunks:** a chunk under 2 seconds inherits the language of the same speaker's previous chunk.
3. **Route each chunk** by its label to that language's model. The `*` route (a multilingual model) takes three cases: other languages, chunks with confidence below 0.6, and chunks where the top two languages are within 0.2 of each other (the mixed case, such as Hindi with English words).
   - **Reuse Hex's reading for English chunks.** When the label is English and Hex's meeting model is an English model, the stage reuses Hex's own reading and makes no new call.
   - **GPU tier only:** low-confidence chunks also get one reading from the language model, and translation receives both readings, as today.
4. **Skip translation for English.** Chunks labelled English with confidence of 0.8 or more are not sent to translation; their text goes straight into the English transcript. An all-English meeting runs no translate step at all.
5. **Keep the original.** The transcript becomes one content blob, `transcript.segments.v1.json`. Each segment holds `start`, `end`, `track`, `speaker`, `lang`, `lang_conf`, `text_original`, `text_en`, and the provider and model for each step. The UI offers an English view and an Original view.
6. **Cache every result.** Results go in `local/cache/<step>/<key>.json`, which is device-local, never syncs, and is capped at 500 MB (least recently used goes first).
   - **Cache key:** `sha256(input bytes or text, provider, model id or checksum, language, prompt template version)`.
   - **Effect:** re-running notes re-reads nothing, and changing the Hindi route re-reads only the Hindi chunks.

### (c) The model directory

- **A curated file, changed only by pull request.** The catalog is `assets/catalog/models.toml`, one `[[model]]` per entry. Each entry has these fields:
  - identity and tasks: `id`, `name`, `tasks` (lid, asr, translate, llm), `languages`;
  - size: `size_mb`, `params`;
  - licence: `licence` (SPDX), `licence_url`, `commercial_use`, `gated`;
  - install: `runtime` (hex, sherpa-onnx, ctranslate2, endpoint), `install` (a Hex id, or file URLs with a `sha256` each);
  - hardware: `min_ram_mb`, `min_vram_mb`, `cpu_ok`;
  - provenance: `source_url`, `verified_on`.
- **CI checks every pull request** against the schema: SPDX licence ids, a hash for every download, and no duplicate ids. Community additions arrive as pull requests that follow `docs/models/adding-a-model.md`.
- **The catalog ships inside each release.** "Refresh list" fetches the latest file only when the user clicks it.
- **Search and filter.** Settings > Models searches name, language and task, and filters by task, language, size, licence and "Fits this PC". The fit badges follow LM Studio's pattern:
  - "Fits GPU";
  - "Runs on CPU, slower";
  - "Too large".
- **Install speech models through Hex.** Before calling `hex model install`, Meetings reads `transcription.model` and restores it with `hex model use` afterwards, so dictation never changes. An upstream `--keep-selection` flag would remove this step.
- **Download other models with checks.** Meetings downloads the listed files, checks each `sha256`, and stores them in `local/models/`.
- **Gated models are never fetched by the app.** Some models require accepting terms on Hugging Face first. For those the app shows "Open model page" instead of Install.
- **Show the licence before install.** The install dialog always shows the licence name, a link, "Commercial use: yes or no" and the size. Install is the only button that downloads anything. Nothing downloads at startup, on search or on update.
- **"Add your own" has four paths:**
  1. A GGUF speech model, through `hex model import`. The user then picks which languages to route to it.
  2. An OpenAI-compatible URL. It is probed with `/v1/models`, and its models are listed.
  3. An ACP agent command.
  4. A CLI agent.

  These entries live in the device-local `models.user[]` list, labelled "Added by you, licence not checked".

**Seed entries:**
- **Language detection:** sherpa-onnx Whisper tiny and base. The candidate SpeechBrain VoxLingua107 ECAPA (Apache-2.0, 6.7% error on its dev set, 107 languages) enters only if lane M5 shows that it beats Whisper on short chunks.
- **Speech:** Hex's catalog mirrored, plus AI4Bharat IndicConformer 600M (MIT, 22 Indian languages, gated).
- **Translation:** IndicTrans2 Indic to English, the 200M distilled and 1.1B versions (MIT), and MADLAD-400 3B (Apache-2.0).
- **Rejected as defaults:** NLLB-200 and MMS-LID, both CC-BY-NC-4.0, so they would be listed with "Commercial use: no" at most. Parakeet v3 and Canary v2 cover 25 European languages and no Hindi.

### (d) Settings UI (phone-width, one column)

**Settings > Pipeline.**
- At the top:
  - "Meeting languages", as chips with "+ Add".
  - "This PC: GPU 6 GB, Standard tier" with a "Use recommended" button.
- Then eight rows, one per step. Each row shows the step name, "Uses: Claude Code" and a small "If it fails: Ollama".
- Tapping a row opens a sheet with:
  - one provider picker, grouped as Hex models, Local models, Local servers and Agents;
  - a model field;
  - a fallback picker;
  - a Skip switch, where the step allows it.
- The Speech sheet shows the routes as rows, for example "Hindi → Nemotron 3.5". The Language row shows "Detects: Hindi, English".

**Settings > Models.**
- A search box, one row of filter chips, then cards.
- Each card shows the name, tasks, languages (first 3, then "+N"), size, a licence badge, a fit badge and one button: Install, Installed or Use.
- Two tabs: Directory and Added by you. "Add your own" sits at the bottom of the second tab.

### (e) Efficiency targets

Lane M0 measures the baseline, and lanes M5 to M7 must meet these targets:

| Measure | Target |
| --- | --- |
| Extra time for language detection, per hour of audio | 60 s or less on CPU (Standard), 15 s or less on GPU |
| Average recognition calls per chunk | 1.15 or fewer (today 2, plus Hex's own reading) |
| LLM input for English chunks | 0 tokens |
| Re-running notes only | no audio work; under 5 s before the agent call |
| Quick title | within 30 s of the meeting's end, from Hex's live transcript |
| Total audio stage on the GPU tier | under 0.15 × meeting length |

**Steps are skipped in these cases:**
- Meeting languages are English only: no detection, no new recognition (Hex's transcript is used) and no translation.
- A chunk under 2 seconds: no detection (it inherits the previous label).
- Memory is off by default.

### (f) Migration and fit

1. **No change until the user switches.** Lane M1 adds the `pipeline` section with a `compat` profile that matches today's scripts. The owner's runs only change when they pick "Use recommended".
2. **Ride ADR 0006's runner.** Lane H3's runner calls the scripts as stages. Lane M5 adds `language_detect` as a Rust stage and depends on lane H7's sherpa-onnx decision. Lanes M6 and M7 replace the transcribe and translate scripts.
3. **LLM steps go through `Harness`.** `EndpointHarness` joins `AcpHarness` and `CliHarness`. Endpoints are loopback-only by default (see owner choice O1).
4. **Storage stays behind ADR 0001's seam.** Segments, both texts and per-step Provenance are a Transcript content blob behind `Store::put_blob`. Config, user models, `local/models/` and `local/cache/` are device-local and never sync.

## Owner choices

- **O1. Endpoints off this machine.**
  - (A) Loopback only. This is the default and keeps ADR 0006's "no cloud APIs" rule.
  - (B) Any URL, after a warning naming the host that receives meeting text.
- **O2. The translation default for Indian languages.**
  - (A) The user's agent. This is the default: it handles mixed Hindi and English best and uses the glossary.
  - (B) IndicTrans2, local and free, but trained on clean text.

  Decide after lane M11 runs on real meetings.

## Options rejected

- **One multilingual model for everything:** Whisper turbo misreads mixed speech and detects the language once per file. It stays as the `*` route.
- **Running every model on every chunk (today's design):** it costs about 3 readings per chunk.
- **MMS-LID:** CC-BY-NC, and a 1B-parameter model for a 2-way choice.
- **Fetching the catalog live from Hugging Face:** it brings no licence review, no hashes and network calls without a click.
- **A separate meeting model setting in Hex:** routing needs per-chunk choice, which the local API already allows.

## Sources

- Whisper detects the language once per file: https://github.com/openai/whisper/blob/main/whisper/transcribe.py
- sherpa-onnx language identification: https://k2-fsa.github.io/sherpa/onnx/spoken-language-identification/pretrained_models.html
- SpeechBrain VoxLingua107 ECAPA: https://huggingface.co/speechbrain/lang-id-voxlingua107-ecapa
- MMS-LID: https://huggingface.co/facebook/mms-lid-126
- AI4Bharat IndicConformer: https://huggingface.co/ai4bharat/indic-conformer-600m-multilingual
- Parakeet v3: https://huggingface.co/nvidia/parakeet-tdt-0.6b-v3
- Canary v2 and Parakeet v3 report: https://arxiv.org/abs/2509.14128
- IndicTrans2: https://arxiv.org/pdf/2305.16307
- NLLB licence: https://github.com/facebookresearch/fairseq/blob/nllb/README.md
- MADLAD-400: https://huggingface.co/google/madlad400-3b-mt
- Ollama OpenAI compatibility: https://docs.ollama.com/api/openai-compatibility
- llama.cpp server: https://github.com/ggml-org/llama.cpp/blob/master/tools/server/README.md
- ACP agent list: https://agentclientprotocol.com/get-started/agents
- Ollama library: https://ollama.com/library
- LM Studio staff picks: https://lmstudio.ai/changelog/lmstudio-v0.3.16
- Jan model management: https://jan.ai/docs/manage-models
- Hugging Face model cards: https://huggingface.co/docs/hub/model-cards

**Verify in the lane that first depends on each claim:**
- whether sherpa-onnx returns per-language probabilities (needed for the restriction and for the margin);
- `json_schema` support in each endpoint;
- the LM Studio port and its `/v1/models` behaviour.
