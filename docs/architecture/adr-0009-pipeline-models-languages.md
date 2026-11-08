# ADR 0009: Configurable pipeline, language routing and model directory

Status: proposed, 2026-11-08; amended the same day with the owner's decisions and the Indic ASR research (see Sources). Amends ADR 0006 (adds a provider per stage and an `EndpointHarness`). Every model size, speed and quality figure below is **unverified** until lane M0 measures it.

## Context

- **Today's scripts read every chunk three times.** `meeting-audio.py` splits each track into speech chunks of 24 seconds or less and sends each to two language-specific Hex models (`POST /transcriptions?model=&language=`). An LLM merges those readings with Hex's own English reading and translates in batches of about 18,000 characters.
- **What Hex offers:** a 17-entry speech catalog, `model import` for GGUF files, `model transcribe --model --language` for WAV files, and `model devices`. **Its limits:**
  - Meetings and dictation share one model (`transcription.model`). Upstream issue [anomalyco/hex#1351](https://github.com/anomalyco/hex/issues/1351) asks for a separate meeting model.
  - No language detection; Whisper-family models accept `auto`.
  - `model install` downloads a model **and selects it**, which changes the user's dictation model.
  - `model transcribe --model <id>` loads an installed model for one call and saves no selection ([cli.rs:681-740](https://github.com/anomalyco/hex/blob/dev/src/control/cli.rs#L681-L740); `save_selection` is separate, at [cli.rs:643-658](https://github.com/anomalyco/hex/blob/dev/src/control/cli.rs#L643-L658)). This is source reading, not a live test.
  - Upstream [Windows docs](https://github.com/anomalyco/hex/blob/dev/docs/windows.md) say the local transcription service is unavailable on Windows; the owner's build may differ, and M0 checks.

**Owner decisions, 2026-11-08.**
- **Everything builds on Hex.** Each language's default model is small and fast on CPU, ideally 100 to 300 MB like Parakeet. Bigger models are opt-in (more RAM or a GPU).
- **Sidenote uses its own model, separate from dictation.** Now: explicit `--model` plus GGUF import. Later: Hex alone, once #1351 and a non-selecting install ship.
- **Small non-English models that Hex cannot load run through sherpa-onnx in Sidenote** (provisional).
- **Users declare their meeting languages and can force a route**, for example "Hindi + English mixed".

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
| Hex model | `hex:<model-id>` | Hex local API or `hex model transcribe --model` |
| Local model runtime | `runtime:<catalog-id>` | sherpa-onnx (provisional, see (c)) or CTranslate2 inside the job runner |
| OpenAI-compatible endpoint | `endpoint:<name>` | HTTP; local or remote, the user's own key (see below) |
| ACP agent | `acp:<agent-id>` | ADR 0006 `AcpHarness` |
| CLI agent | `cli:<agent-id>` | ADR 0006 `CliHarness` |

`hex:meeting-transcript` means Hex's own meeting transcript, used as is.

**LLM steps use the `Harness` trait.** Translate, quick title, notes, tasks and memory go through ADR 0006's `Harness`. A third implementation, `EndpointHarness`, calls `/v1/chat/completions` and probes `/v1/models` (Ollama, LM Studio and llama.cpp `llama-server` expose both). It offers no tools, so `ToolPolicy::None` always holds. For JSON outputs it sends `response_format` with a JSON schema, falls back to JSON mode when the server rejects it, and validates the result either way.

**Endpoints (owner decision O1).** The user may connect any OpenAI-compatible endpoint, local or remote (their own server, a cloud GPU, or a provider with zero data retention), for any step, with their own API key. It is a feature for speed, or for a laptop that is too weak. The rules:
- **Keys live in the OS keystore** (lane S5). `config.json` holds only the URL and key name. A key never appears in config, logs or synced files.
- **A one-time notice per endpoint** names the host that will receive meeting text, and audio when a speech step is remote. The first send waits for the user's yes; a changed host asks again.
- **Local is the default for new users.** Nothing is sent until the user picks an endpoint for a step, and "Use recommended" never picks a remote one.
- **Remote URLs must use https.**
- **ACP and CLI agents remain the official adapters;** the generic endpoint covers everything else. This amends ADR 0006's "no cloud APIs" rule for endpoints the user chooses.

**Audio steps use two traits:** `LanguageId::identify(chunk) -> (lang, confidence, top2)` and `SpeechEngine::transcribe(chunk, lang) -> (text, confidence)`.

**Fallbacks.** Every step has one `fallback` provider, tried once when the chosen one fails, then ADR 0006's retry rule applies.

**Config lives in device-local `config.json`** (ADR 0001 class 4; it never syncs). Model ids below are examples, and every route is provisional until lane M0 picks it:

```json
"pipeline": { "schema": 1, "profile": "recommended", "meeting_languages": ["hi", "en"],
  "language_mode": "auto",
  "endpoints": { "cloud-gpu": { "base_url": "https://llm.example.com/v1", "key": "keystore" } },
  "steps": {
    "language_detect": { "provider": "runtime:whisper-lid-base", "fallback": "off" },
    "speech": { "routes": { "en": "hex:parakeet_unified_en", "hi": "hex:nemotron_3_5_asr",
                            "ta": "runtime:indicconformer-ta-ctc",
                            "*": "hex:whisper_large_v3_turbo" }, "fallback": "hex:meeting-transcript" },
    "translate": { "provider": "acp:claude-code", "fallback": "endpoint:ollama", "skip_english": true } } }
```

`language_mode` is `auto` (detect per chunk), `off` (first meeting language's route) or `forced:<route>` (every chunk takes one route, such as `forced:hi+en`).

**Hardware tiers** come from `hex model devices`; "Use recommended" applies the tier's defaults. Bigger models are opt-in at every tier.

| Tier | Hardware | Default behaviour |
| --- | --- | --- |
| Light | CPU only, under 16 GB RAM | No language detection; Hex's meeting transcript as is; agents do the text steps. For non-English languages the app offers an explicit compact route (see (c)) and never skips Indic recognition silently. |
| Standard | 16 GB RAM or more, or a GPU under 6 GB | Anuvani language detection when a second language is declared; one routed recognition pass. |
| GPU | 6 GB VRAM or more | As Standard; endpoints offered. A second reading for unclear chunks is unmeasured and off. |

### (b) Language handling

Updated 2026-11-09 from measurements. Sources: `reviews/hex-meetings-2026-11-08/m0b-hindi-quality.md` (7 model setups, 50 public clips per class) and `language-routing.md` (6 detectors, 100 clips per class). Numbers are word error rates (WER) in points, on CPU, under load.

**Why routing decides accuracy.** Every model is good on one class and unusable on the others:

| Model | Indian English | Hindi | Hindi-English mixed |
|---|---:|---:|---:|
| Parakeet English (Hex default) | 7.2 | 99.1 | 78.2 |
| IndicConformer Hindi (198 MB, sherpa-onnx) | 107.2 | 12.8 | 47.4 |
| Nemotron 3.5 (in Hex) | 105.9 | 18.0 | 43.8 |

Routing each chunk with the Anuvani detector between Parakeet and Nemotron gave 23.8 pooled WER, against 56.0 for always Parakeet and 59.9 for always Nemotron. Mixed-speech WER is scored across scripts and is approximate: English words written in Devanagari count as errors.

1. **Declare the meeting's languages,** for example English, or English and Hindi. The default is English only, which runs no detector and no Indic model, so English-only users pay nothing. The user can **force a route** (`language_mode` = `forced:en`, `forced:hi`, `forced:mixed`) for a meeting, which skips the detector.
2. **Every chunk gets the English reading first.** Parakeet runs on the whole meeting as today (real-time factor 0.069). Routing only replaces chunks, so a detector failure or a missing model leaves the English transcript, marked "language not checked".
3. **Detect per chunk** when more than one language is declared.
   - **Detector: Anuvani INT8** (CIIL, Apache-2.0, 169 MB, lists Indian English and Hindi). Measured: 8% of English clips sent to Hindi, 1% of Hindi and 5% of mixed clips missed. The next best, Whisper base, missed 17% of Hindi and 11% of mixed clips. Whisper tiny, the old default here, missed 18% and 26%. SpeechBrain sent 74% of Indian English to Hindi and is rejected.
   - **Cost:** 12.2 seconds of one CPU thread per audio minute, about 12 minutes for a one-hour meeting, after the meeting. It runs only on speech chunks and only for users who declare a second language. Multithreading is unmeasured.
   - **Decision rule (provisional):** Hindi when Anuvani's Hindi score, restricted to the declared pair, is above 0.9. On the test clips this gave 1% false Hindi, 5% Hindi miss and 18% mixed miss, but it was chosen on those same clips, so it is not validated. The threshold lives in one constant.
   - **Units:** VAD chunks of 24 seconds or less, pooled within a speaker turn when speaker labels exist. A chunk under 2 seconds takes the decision of the same speaker's neighbouring chunks; never across speakers. Smoothing is unmeasured.
   - **A label is not proof.** The detector separates English from Hindi; it does not detect mixed speech. Low margins are not a mixed-speech signal.
4. **Route non-English chunks to the declared Hindi model.**
   - **"Hindi" declared:** IndicConformer Hindi (198 MB download, real-time factor 0.17 on CPU): 12.8 WER on Hindi, 5.2 points better than Nemotron.
   - **"Hindi and English mixed" declared:** Nemotron through Hex (`hex model transcribe --model`, never `hex use`): 43.8 on mixed, 3.6 points better than IndicConformer. If Nemotron is not installed in Hex, IndicConformer takes the chunk.
   - **Batch per route.** Chunks for one model are joined into one WAV with an offsets table, and each model loads once per meeting. Loading per chunk cost about 80 seconds per call in M0b.
5. **Show and override.** Each segment shows its route. The user can change a speaker turn's route and rerun only that turn.
6. **Skip translation for English.** Chunks routed English skip translation. An all-English meeting runs no translate step.
7. **Default status.** Automatic routing ships as "experimental" until it is checked on held-out real meeting audio with Hindi in it (the owner's meetings so far were all English). Hex does not expose a Parakeet confidence score, and a text-based proxy failed (47% false Hindi), so confidence-based routing is not used.
8. **Keep the original.** The transcript is one blob, `transcript.segments.v1.json`; each segment holds times, track, speaker, `lang`, `lang_conf`, `text_original`, `text_en` and the provider and model per step. The UI offers an English view and an Original view.
9. **Cache every result** in `local/cache/<step>/<key>.json` (device-local, 500 MB cap, least recently used first). The key is `sha256(input, provider, model id or checksum, language, prompt template version)`, so re-running notes re-reads nothing and changing the Hindi route re-reads only Hindi chunks.

### (c) The model directory

- **A curated file, changed only by pull request.** The catalog is `assets/catalog/models.toml`, one `[[model]]` per entry. Fields: `id`, `name`, `tasks`, `languages`, `size_mb`, `params`, `licence` (SPDX), `licence_url`, `commercial_use`, `gated`, `runtime` (hex, sherpa-onnx, ctranslate2, endpoint), `install` (a Hex id, or file URLs with a `sha256` each), `min_ram_mb`, `min_vram_mb`, `cpu_ok`, `source_url`, `verified_on`. Three more, `cpu_rtf`, `wer_hi_en` and `measured_on`, stay `null` (shown as "unverified") until M0 fills them.
- **CI checks every pull request:** SPDX ids, a hash for every download, no duplicate ids (`docs/models/adding-a-model.md`). The catalog ships in each release; "Refresh list" fetches the latest file only on a click.
- **Search and filter** by name, language, task, size, licence and "Fits this PC". Fit badges: "Fits GPU", "Runs on CPU, slower", "Too large".
- **Sidenote never changes the user's dictation model.**
  - **Now:** `hex model transcribe --model <id>` on models already installed in Hex, and `hex model import` for new GGUF files (after an architecture and licence check). If a catalog model is not installed, the app tells the user to install it in Hex, because that call switches dictation.
  - **Never** `hex model install` followed by a restore: dictation changes in the gap, a user change can race the restore, and the restore can fail.
  - **Later:** Hex alone, once #1351 and a non-selecting install land. Per-chunk routing stays in Sidenote.
- **Provisional runtime.** Models that Hex cannot load run through sherpa-onnx in Sidenote, from tested ONNX packs only (lane M12). Lane M13 works upstream; Sidenote removes the runtime when Hex loads the models.
- **Downloads are checked and explicit.** Sidenote checks each file's `sha256`, stores files in `local/models/` and never fetches gated models (the app shows "Open model page"). The install dialog shows the licence, a link, "Commercial use: yes or no" and the size. Install is the only button that downloads anything.
- **"Add your own"** takes a GGUF model (`hex model import`), an OpenAI-compatible URL and key, an ACP command or a CLI agent. Entries sit in the device-local `models.user[]` list, labelled "Added by you, licence not checked".

**Seed entries:**
- **Language detection:** Anuvani INT8 (169 MB, Apache-2.0), measured best in (b). Whisper tiny and base stay as smaller fallbacks. SpeechBrain VoxLingua107 is rejected: it sent 74% of Indian English to Hindi.
- **Speech:** Hex's catalog mirrored, plus the candidates below. IndicConformer 600M (MIT, 22 languages, gated) is listed but not a small default.
- **Translation:** IndicTrans2 Indic to English (MIT) and MADLAD-400 3B (Apache-2.0).
- **Rejected as defaults:** Shuka-1 (9B); NLLB-200 and MMS-LID (CC-BY-NC-4.0); Parakeet v3 and Canary v2 (no Hindi).

**Provisional model tiers.** Sizes are downloads, not RAM. Hindi, mixed and Indian English WER for Parakeet, Nemotron, IndicConformer Hindi, Whisper turbo and Swift is now measured on public clips (see (b)); real meeting audio and the other languages are still unmeasured. Measured Hindi defaults: IndicConformer Hindi for Hindi, Nemotron for mixed; Swift scored 58.4 on mixed and 91.6 on Hindi and is opt-in only. No source shows any candidate matching Parakeet's speed. The two Hex-native candidates (Nemotron, Parakeet) exceed 300 MB and are listed because Hex loads them today.

| Language group | Small default candidate | Medium, opt-in | Large, opt-in |
| --- | --- | --- | --- |
| Hindi / Hinglish | [Nemotron 3.5](https://huggingface.co/Litovation/nemotron-3.5-asr-streaming-0.6b-gguf) Q4, 496 MB, in Hex; or [Whisper Hindi2Hinglish Swift](https://huggingface.co/Oriserve/Whisper-Hindi2Hinglish-Swift) ONNX, about 150 MB, sherpa-onnx | [Qwen3-ASR 0.6B](https://huggingface.co/Qwen/Qwen3-ASR-0.6B) Q8, 850 MB | Nemotron 3.5 F16, 1.28 GB |
| Other Indian languages | [IndicConformer](https://huggingface.co/parismitaglobalsolutions/indicconformer-sherpa-onnx) CTC ONNX, about 190 MB per language, sherpa-onnx | Whisper small ONNX, about 360 MB | [omniASR CTC 300M](https://huggingface.co/facebook/omniASR-CTC-300M/tree/main), 1.3 GB, runtime unverified |
| English | Hex Parakeet unified Q4, about 477 MB ([listing](https://huggingface.co/cswk/sayit-asr-gguf)) | Nemotron Q4, 496 MB | Nemotron F16, 1.28 GB |

- Language packs add up (3 at 190 MB is 570 MB); the app shows the total before download. The 150 MB and 190 MB figures are publisher claims.
- Convert to GGUF only when frontend, tokenizer, tensor names and decoder match a family that Hex's engine supports ([conversion guide](https://github.com/handy-computer/transcribe.cpp/blob/main/docs/tools/conversion.md)). A NeMo format or a GGUF header that loads does not show inference works.
- M0 tests Nemotron first, and the other two only if Nemotron fails on CPU speed or Hinglish quality.

### (d) Settings UI (phone-width, one column)

**Settings > Pipeline:** meeting-language chips, a "Language handling" choice (detect per chunk, off, or a forced route such as "Hindi + English mixed"), "This PC: GPU 6 GB, Standard tier" with "Use recommended", then eight step rows. A tap opens a sheet with a provider picker, model field, fallback picker and Skip switch. "Add endpoint" takes a URL and key and shows the host notice. **Settings > Models:** search, filter chips and cards (name, tasks, languages, size, licence, fit badge, "unverified" or measured CPU speed, one Install, Installed or Use button), in tabs Directory and Added by you.

### (e) Efficiency targets

Lane M0 measures the baseline plus WER (mixed-script normalization) and CPU real-time factor per candidate. Lanes M5 to M7 must meet these targets, which stay provisional until M0 reports; M0 also sets the CPU real-time factor each language default must meet.

| Measure | Target |
| --- | --- |
| Extra time for language detection, per hour of audio | 60 s or less on CPU (Standard), 15 s or less on GPU |
| Average recognition calls per chunk | 1.15 or fewer (today 2, plus Hex's own reading) |
| LLM input for English chunks | 0 tokens |
| Re-running notes only | no audio work; under 5 s before the agent call |
| Total audio stage on the GPU tier | under 0.15 × meeting length |

### (f) Migration and fit

1. **No change until the user switches.** Lane M1 adds the `pipeline` section with a `compat` profile that matches today's scripts. Runs change only when the user picks "Use recommended".
2. **Ride ADR 0006's runner.** Lane H3's runner calls the scripts as stages. Lane M5 adds `language_detect` (after H7's sherpa-onnx decision), lanes M6 and M7 replace the transcribe and translate scripts, and lane M12 adds the provisional runtime.
3. **Retire the extra runtime only at parity.** Lane M13 (upstream Hex work) targets a meeting-model setting and a non-selecting install. Sidenote removes `runtime:` speech entries only after Hex loads those models, and a migration moves the user's routes to `hex:` ids.
4. **LLM steps go through `Harness`.** `EndpointHarness` joins `AcpHarness` and `CliHarness`. Endpoints follow decision O1: any URL, keys in the keystore, a notice before the first send.
5. **Storage stays behind ADR 0001's seam.** The segments blob (both texts, per-step Provenance) goes behind `Store::put_blob`. Config, user models, `local/models/` and `local/cache/` stay device-local.

## Owner decisions on open choices

- **O1. Any OpenAI-compatible endpoint, local or remote. Decided.** Users connect their own server, a cloud GPU or a zero-retention provider, with their own key, per step. The rules are under "Endpoints" in (a).
- **O2. Indian-language translation. Decided provisionally:** the user's own agent translates (best on mixed Hindi and English). IndicTrans2 stays the alternative. Revisit after M0.

## Options rejected

- **Fetching the catalog live from Hugging Face:** no licence review, no hashes, calls without a click.
- **Install, then restore the dictation model:** see (c).
- **A meeting-model setting in Hex as the whole answer:** per-chunk routing stays in Sidenote; #1351 retires only the extra runtime.
- **Treating a language label as proof of one language, claiming small means fast, assuming any GGUF model loads:** code switches occur inside chunks, no CPU measurement exists yet, and architecture must match.

## Sources

- Whisper detects the language once per file: https://github.com/openai/whisper/blob/main/whisper/transcribe.py
- sherpa-onnx language identification: https://k2-fsa.github.io/sherpa/onnx/spoken-language-identification/pretrained_models.html
- SpeechBrain VoxLingua107 ECAPA: https://huggingface.co/speechbrain/lang-id-voxlingua107-ecapa
- AI4Bharat IndicConformer: https://huggingface.co/ai4bharat/indic-conformer-600m-multilingual
- Parakeet v3: https://huggingface.co/nvidia/parakeet-tdt-0.6b-v3
- Canary v2 and Parakeet v3 report: https://arxiv.org/abs/2509.14128
- IndicTrans2: https://arxiv.org/pdf/2305.16307
- NLLB licence: https://github.com/facebookresearch/fairseq/blob/nllb/README.md
- MADLAD-400: https://huggingface.co/google/madlad400-3b-mt
- Indic ASR research, 2026-11-08 (source reading, no live measurement): `E:\Personal-Projects\reviews\hex-meetings-2026-11-08\indic-asr-research.md`
- Ollama OpenAI compatibility: https://docs.ollama.com/api/openai-compatibility
- llama.cpp server: https://github.com/ggml-org/llama.cpp/blob/master/tools/server/README.md
- LM Studio staff picks: https://lmstudio.ai/changelog/lmstudio-v0.3.16

**Verify in the lane that first depends on each claim:** whether sherpa-onnx returns per-language probabilities; `json_schema` support in each endpoint; the LM Studio port; and, per ASR candidate, licence, gated access, Windows CPU speed and which runtime loads it.
