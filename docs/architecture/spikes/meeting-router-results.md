# Meeting language routing proof

Decisions needed: none. Automatic routing remains opt-in and experimental. This evidence uses public clips and isolated state, not the installed listener or private meetings.

## Stack and contracts

The stack starts at catalogue main (`cbc6a5a`): `asr-runtime`, `router-core`, `router-jobs`, then `router-ui`. Runtime constructors use the catalogue snapshot for `anuvani-lid-int8` and `indicconformer-hi-ctc`; file-open/load failures remain ordinary unavailable-model failures. The runtime pins the documented DLL SHA256 before loading. Saved languages have one catalogue-owned store; pipeline config stores only the mode and runtime configuration.

The core keeps baseline English segments separately, with their original track and intervals. Routed windows replace a baseline interval only when their union fully covers it. A partially covered sentence stays whole in English, marked. The full baseline also remains in the blob when replacement succeeds. Grouped text is never split by guessed word times.

Artifact identity includes source-file paths, sizes and mtimes, baseline English content, the window set, declared languages, routing version, graph/token checksums and runtime checksum. Successful artifacts may be reused; any marked fallback is degraded and remains retryable. Per-window caches retain audio/provider/model/language identities. Model paths still have no inference lease.

Audio is read through seekable PCM16 WAV readers. Only one bounded window or speaker-turn pool expands to f32. Route WAVs are written incrementally, rather than retaining full tracks and joined float copies. IndicConformer holds one session across all Hindi windows, including Hindi fallback after mixed-route failures.

Mixed windows form separate groups: consecutive mixed windows on the same track join only when their gap is under three seconds. An intervening English window breaks the group. Every group has its real start/end and source offsets. Segments remain in time order. Each uncached group invokes explicit-model Hex transcription; no selection-changing command is used.

## Method and accuracy

`tools/evaluate_meeting_router.py` reran the same first 50 matched public clips in each class, joined per class with half-second gaps. It supplies the actual saved Parakeet hypotheses, not references. The scorer uses the same NFKC/digit/Latin ITRANS normalization and jiwer weighting as `lid/score-routing.py`. Cross-script WER is approximate. Every scored run accounts for all 50 class-scoped IDs exactly once; IDs repeat across classes.

| Profile, same 150 clips | English WER | Hindi WER | Mixed WER | Pooled WER |
|---|---:|---:|---:|---:|
| Published Parakeet/Nemotron routing | 10.0 | 17.4 | 42.2 | 23.8 |
| Previous Sidenote, English/Hindi pair throughout | 7.15 | 12.36 | 45.48 | 22.67 |
| Final core, English/Hindi pair throughout | 7.15 | 12.36 | 45.48 | 22.67 |
| Previous Sidenote, mixed declared for mixed class | 7.15 | 12.36 | 42.68 | 21.63 |
| Final core, mixed declared for mixed class | 7.15 | 12.36 | 41.12 | 21.06 |

The final pair has 394 edits / 1,738 reference words. The mixed-declaration profile has 366 / 1,738. English and Hindi WER are unchanged; all 49 native Hindi hypotheses exactly match the previous run. Hindi stays within 0.44 points of the 12.8 reference. Forced mixed remains 44.55 WER on the mixed class.

The mixed change exceeds the one-point rerun tolerance, so it was investigated. All 50 route assignments and all English hypotheses match the previous run. The same 35 windows go to mixed and 15 stay English. The mandated grouping creates 13 Hex calls instead of one. Against the identical concatenated non-English reference, the old text has 204 edits and the new text 194. Scoring the new groups separately also gives 194: zero difference comes from scoring boundaries. The 10-edit improvement is transcription context changing between one disjoint batch and 13 contiguous groups, not a detector change (`comparison.json`). The mixed declaration is prior user knowledge, not three-class detection.

## Time, RAM and load cost

Final core processes ran at normal priority, with no sampled cargo/rustc process. CPU/RAM sampling covers the process tree every 100 ms. Audio minutes exclude added join gaps. Routing rows reuse the English reading and are incremental after that pass.

| Workload | Wall s / audio min | CPU s / audio min | Peak MiB |
|---|---:|---:|---:|
| English only, existing reading exported | 0.28 | 0.18 | 13.3 |
| Fresh Parakeet whole-file reading | 3.61 | 2.45 | 1,199.7 |
| English/Hindi auto, English audio | 21.18 | 18.82 | 457.7 |
| English/Hindi auto, Hindi audio | 35.90 | 32.76 | 613.9 |
| Auto with mixed declared | 40.37 | 33.22 | 1,072.7 |
| Forced mixed, no detector | 17.38 | 16.10 | 453.4 |

These are dev builds and include model-file checksumming that a later lead commit removed (models are now identified by catalogue hash, see `cache::installed_identity`), so these totals overstate current cost; they were not re-measured. Fingerprinting is not separately profiled; the additional checksums and repeated mixed startup increase short-job cost. Do not interpret these totals as pure inference speed or linearly extrapolate repeated startup to an hour. English-only app users skip the new job. Uniform mixed/Hindi accuracy reused the detector cache; its warm timing is not a fresh route benchmark.

Anuvani loads were 2.746 s (English), 2.895 s (Hindi) and 2.856 s (mixed); Hindi loaded in 1.572 s. The 13 mixed Hex calls consumed 61.943 s in total, including selection checks, startup, decode and shutdown. That is 12 extra calls versus the prior route-wide batch. The earlier quarter-second silence proxy was 4.181 s per Nemotron invocation; multiplying by 12 estimates 50.17 s of repeated startup/decode/shutdown overhead, not isolated model loading. Parakeet's earlier proxy was 4.944 s. Both selections stayed Parakeet. There is no pure Hex load timer through this CLI.

The one-hour memory proof joins only public Hindi clips into a 3,600-second, 115,200,044-byte WAV with 617 windows. Both native sessions load fresh. Repeated windows deliberately reuse the preceding run's cache: 145.706 s wall, 132.766 s CPU, peak 465.2 MiB; detector load 2.307 s, Hindi load 2.107 s. All 617 source IDs remain present. This verifies duration scaling of WAV I/O and cached routing with resident sessions. It is not fresh hour-long ASR throughput, a cold-cache peak, or a one-hour Nemotron memory benchmark. The fresh five-minute Hindi run peaks at 613.9 MiB; transient inference allocations depend on window length, bounded to 24 seconds.

## Artifacts, gates and limits

Evidence is under `<proof-root>/sidenote-router-proof-v2/`: `normal/measurements.json`, `comparison.json`, and `hour/evidence.json`. The final core executable is `artifacts/router-core-final/sidenote.exe`, SHA256 `cef0b4dacb3a211b23bd1612153da81a1547b7acd82970656441aa5a0e06cbe2`. The 150-clip and hour runs use that exact copy. Its dev sizes are 165,496,832 bytes without and 180,738,560 with the backend. The runtime branch's sizes are 164,361,216 / 179,379,712 bytes. The DLL pair adds 17,146,880 bytes and models about 366.7 MB. Release installer size and clean-machine redistributable installation remain unverified.

The DLL integrity run uses that same core executable. The documented pinned DLL returns Hindi, matching the measured native text; a same-size copy with its final byte changed returns marked English fallback, exit zero, and preserves the baseline. It is rejected before loading (`dll-integrity/evidence.json`). The temporary altered copy never replaces the runtime or model store.

Stable fmt, locked clippy with all targets and warnings denied, and locked builds pass in both feature configurations. Logs: `reviews/sidenote-router-build/runtime-integrity-gate-*` and `router-core-final-gate-*`. Cargo stays BelowNormal and uses the shared target. Refresh both main and lib before build, then copy the exe. No tests ran; no merge or push occurred. Audio, transcripts, DLLs and models stay outside git.

Queue supervision and UI are separate stacked changes. The existing notes script remains active and may repeat ASR/translation work; stage timings above do not measure that full legacy workflow. This core marks English `translate: false`, but does not replace the legacy translation runner. The provisional 0.9 detector threshold, energy-VAD fallback, speaker pooling and real-meeting boundary quality remain unvalidated on held-out meeting audio.

Per-turn override/rerun UI and word alignment are explicitly follow-ups, outside this review round. Release packaging, private or held-out meeting quality, cold-cache hour-long ASR and hour-long Nemotron RAM remain unverified. No installed app, listener or scheduled task was started or stopped. Evidence rung 4 applies to CLI execution, scoring and gates; renders only demonstrate their rendered UI.

The stacked jobs branch records queue, crash, normalization, restart and invalidation evidence in [job supervision results](router-jobs-results.md).

The stacked UI branch records routed-only refresh, clean English labels, Settings copy and dark/light renders in [UI evidence](router-ui-results.md).
