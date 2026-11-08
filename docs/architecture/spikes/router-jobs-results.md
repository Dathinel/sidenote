# Meeting job supervision and recovery

Decisions needed: none. The jobs layer adds scheduling around the headless core; it never changes Hex selection or overwrites the English source transcript.

## Queue and process contract

The listener durably writes one `local/router-queue/<meeting>.json` before attempting to launch a worker. Enqueueing does not depend on the worker lock or catalogue settings locks. A single worker holds the drain lock; a busy second worker exits successfully with its request still on disk. A per-request generation and short per-id lock prevent acknowledging a newer enqueue. Unprocessed requests survive parent termination and are recovered by a new worker. Failed setup leaves the request queued and allows other valid requests to proceed.

Automatic recovery polls every 30 seconds for unattempted generations. A completed fallback keeps its queue entry and generation receipt, but is not automatically rerun in a tight loop. An explicit `--route-pending` retries degraded results; `--meeting-id <id>` enqueues a new generation. A model restored before that next drain changes the fingerprint and is loaded again. This policy avoids repeatedly hashing models and rerunning permanently short or partially covered English intervals. There is no implicit historical backfill.

Only the child normalizes audio and runs inference. The supervisor starts it suspended, attaches a kill-on-close Windows job, then resumes it, closing the spawn/containment race. Non-zero exit, termination, missing/invalid output and deadline expiry all cause the parent itself to publish marked English baseline segments. The baseline's track, text and interval survive independently of child output. A deadline is failure even when job termination returns exit zero. See Microsoft's [process creation flags](https://learn.microsoft.com/en-us/windows/win32/procthread/process-creation-flags).

Normalization has a deadline of `clamp(audio_seconds * 0.5 + 15, 30, 7200)` seconds. A valid WAV header supplies the duration; otherwise the baseline duration is used. The full child deadline is `clamp(audio_seconds * 4 + 120, 120, 86400)`. Headless deadline/ffmpeg overrides support controlled evidence runs. Timeout closes the job and kills descendants. Child cleanup guards and parent recovery cleanup remove generated/partial WAVs from the meeting work directory. Input audio remains untouched.

Before reuse, the parent fingerprints audio size/mtime, baseline text and intervals, prepared windows, config/languages and graph/token/DLL checksums. It also checks for changes while the child ran. Any fallback is degraded and is never reused as a successful artifact on the next explicit run. These checks add startup CPU; the parent performs additional checksum scans beyond the core timing table. This was not separately profiled as production throughput.

## Forced failure evidence, rung 4

All runs use copies of public audio and isolated state under `reviews/sidenote-router-proof-v2/jobs-run-1794180532978517600`. No listener or installed application was started. The public baseline interval is 0.000–6.300 seconds. Every fallback below preserves its complete English text and that interval.

| Forced condition | Parent exit | Child/normalizer status | Published result |
|---|---:|---|---|
| Terminate active inference child | 0 | Child exit 15 | One marked English segment; pending request retained |
| Two overlapping meeting requests | 0 for busy worker | Second queue file remains while first worker holds lock | Both artifacts published; second meeting routes Hindi |
| Terminate worker parent before publication | Killed parent 1; new worker 0 | Inference tree killed by job closure | Durable request recovered by a new automatic worker, Hindi |
| Full-child deadline, 1 s | 0 | Child exit 0, `timed_out: true`, unsuccessful | One marked English segment |
| Missing ffmpeg, malformed input | 0 | Child 1; normalizer could not start | One marked English segment; no partial WAV |
| Sleeping fake ffmpeg with descendant, 1 s deadline | 0 | Child 1; normalizer exit 0, `timed_out: true` | One marked English segment; descendant killed; partial WAV removed |

The final timeout row took 48.711 seconds end to end because the parent also fingerprints model files before and after the child. The one-second budget applies to normalization, not those checksum scans. This is a dev-build cost, not a model inference measurement.

Further runs verified that an automatic drain leaves a completed fallback unchanged, while an explicit drain recovers the terminated job to Hindi. A partial window retained the whole English baseline; its cached routed hypotheses were nonempty, so this exercised coverage handling rather than an ASR failure. Full window coverage then recovered Hindi. Changed window metadata, changed baseline text and changed audio mtime each changed the artifact fingerprint. Removing a token table published marked English; restoring it changed the fingerprint, and the next explicit drain produced Hindi and acknowledged the request. Both the original four model files and the four isolated recovery files were hash-checked after restoration.

The assertions and exact exit statuses are in `reviews/sidenote-router-proof-v2/jobs-failure-evidence.json`; `delivery-audit.json` rechecks all 13 evidence cases, all scored source IDs, group intervals, model hashes, renders and the unchanged `parakeet_unified_en` selection. No transcript or audio is committed.

## Build and remaining limits

The copied jobs executable is `artifacts/router-jobs-delivery/sidenote.exe`, SHA256 `804ac50c1396eea6b7ef66c26a37c9fd16406ebaf5f5daf4d0015c5a9ecc70cf`. Dev sizes are 166,107,648 bytes without and 181,312,000 with the backend. Stable fmt, locked clippy with all targets and warnings denied, and locked builds pass in both configurations. Logs are `reviews/sidenote-router-build/router-jobs-delivery-gate-*`, plus the final `jobs-delivery-fmt-check` and `jobs-delivery-default-clippy` checks after the recovery policy was finalized. No tests ran. Cargo remains BelowNormal with the shared target; main and lib are refreshed before executable builds.

The installed listener/legacy notes workflow remains unexercised, and that script may duplicate ASR/translation work. Disk publication errors retain the queued request; they cannot guarantee a new artifact when the destination itself is unwritable. Actual ORT abort injection and release packaging were not performed: forced child termination and parent death exercise the supervision boundary without loading a dangerous graph. Per-turn rerun UI and word alignment remain follow-ups outside this round.
