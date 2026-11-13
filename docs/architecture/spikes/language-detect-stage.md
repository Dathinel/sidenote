# Standalone M5 language stage

Decisions needed: none to compile this stage. Runtime qualification remains pending.

The release/dev21 base (cd97099) lacks the H3 stage runner and H7 audio-preparation
decision. This stage accepts explicit prepared mono 16 kHz PCM16 WAVs and chunk
metadata. It does not connect to the listener or change transcription. M6 owns routing.

Enable **Settings > Labs > Language detection (experimental)**, default off. The
standalone CLI reads an explicit config snapshot; it never discovers the installed
app's state. `labs.language_detection` gates inference independently of the existing
router profile. `pipeline.language_mode` supplies `auto`, `off` or `forced:<route>`.
LID settings live under Labs so changing them preserves the existing ASR configuration/cache identity.
Forced routes must use declared languages; `forced:mixed` means the English/Hindi
pair. Overrides have no fabricated confidence. A single declared language skips inference.

Example explicit config snapshot (paths must be absolute on the run machine):

```json
{"labs":{"language_detection":true,"language_detection_settings":{
 "model":"E:/public/anuvani.onnx","runtime_dll":"E:/public/onnxruntime.dll",
 "confidence_threshold":0.9,"margin_threshold":0.2,"silence_mean_square":0.00001}},
 "pipeline":{"language_mode":"auto"}}
```

These three thresholds are **uncalibrated configuration values**, not production
accuracy claims. The graph hash is pinned to the public runtime proof; its read
handle denies modification/replacement through session lifetime. The existing
Windows ONNX DLL loader and its hash check are unchanged, with one intra-op and
one inter-op thread. Missing/mismatched files and unavailable Windows/runtime builds
produce unchecked labels and reasons rather than recognition calls or downloads.

Request example:

```json
{"meeting_languages":["en","hi"],"jobs":[{"id":"public-clip",
 "wav":"E:/public/clip.wav","chunks":[{"id":"one","start":0,"end":3,
 "track":"mic","speaker":"speaker-a","turn":"turn-one"}]}]}
```

```powershell
& $exe --language-detect $request --lid-config $config --lid-output $output
```

On Windows all stage work runs on a named worker at below-normal thread priority. It seeks at most
24 seconds of audio per chunk; it never expands a whole meeting. A <2-second chunk
inherits only a checked predecessor with matching track, known speaker and turn,
and a gap at most 1.5 seconds; otherwise it gets the first declared language with
no confidence. Silence also gets an unchecked fallback. Energy alone cannot reject
music. Unsupported declared languages skip detection rather than shrinking the set. The
unrestricted winner covers only Anuvani's 23 labels; it is not reliable rejection
of an unsupported spoken language.

The output is `$output/<job-id>/language-detect.v1.json`. Each chunk has its ID, source timing, track, speaker/turn,
language, nullable confidence, up to two declared-language scores, status and optional
forced route. Full probability vectors remain device-local cache evidence. Confidence
is the mean sub-window softmax normalized over the declared set, not calibrated
correctness. Five-second windows (with a short tail merged into its predecessor)
retain confident observed winners separately. Low margin is uncertainty, never a
mixed label. An unrestricted winner outside the declared set flags an out-of-set guess.

Caches use PCM content, graph hash, declared languages, settings and algorithm version;
only validated raw successful evidence is reused. Short-chunk inheritance and modes are
recomputed, so predecessor changes cannot reuse a stale decision. Corrupt evidence is
recomputed. Failures are retryable on a later invocation. Cache files have a 500 MiB LRU
cap under the explicit output directory. Sidecars use the existing atomic JSON writer.

## Public benchmark (not a test)

`tools/lid_fleurs_benchmark.py` consumes the proof's `lid-data/sources.json` and
40 public FLEURS windows. FLEURS is CC BY 4.0, from
[google/fleurs](https://huggingface.co/datasets/google/fleurs); the proof pins the
120 dev archive members per language. No audio is committed or downloaded by this tool.

On an **owner-approved Windows machine**, using a compiled asr-runtime binary:

```powershell
python tools/lid_fleurs_benchmark.py --approved-run --exe $exe --data $publicData --model $model --dll $dll --output $newBenchmarkDirectory
```

Never execute this benchmark on the laptop. A Linux boxes.dev host without the pinned
Windows ONNX runtime cannot run this implementation. No remote measurement is claimed.
The script reports accuracy for English, Hindi and Telugu (120 five-second clips each),
and both-language detection plus false extra labels on 60 artificial ten-second mixed
chunks. It splices alternating public Hindi/English clips, not natural code-switching.
All three languages are declared, so this also measures competition beyond a binary pair.
Accuracy counts the top declared label; uncertain and out-of-set counts are reported
separately. Correct top labels are not a qualification to skip ASR in M6.

After model load, the worker signals and waits. The parent samples process working set
as the post-load/pre-inference baseline, starts inference, and samples every 25 ms.
`benchmark.json` reports incremental peak and whether it is below 100 MB (decimal),
separately from total working set. Sampling can miss brief peaks. Output directories
must be new so inference cannot hit a previous cache. Timeout/failure/unavailable runtime
produces no success report. Neither 95% accuracy, memory nor speed has been established
for this stage. Held-out calibration and owner audio follow later; they do not block M5 code.

## M6 handoff

The owner reports that the 64-minute call ran Telugu and Hindi ASR over all 604
segments, spending 621 and 902 seconds respectively. These are owner-supplied
timings, not measurements from this stage. M6's payoff is skipping Indic ASR for
English segments. M5 itself neither invokes recognition nor claims that saving.

Sidecars include the job and source WAV, every chunk's ID, start/end, track,
speaker, turn and optional audio_start. Inspected chunks carry pcm_f32le_sha256,
computed from normalized mono PCM16 samples converted to f32 little-endian using
the exported pcm_sha256 helper. M6 must verify the current input identity before
reusing a guess. Inherited short labels include inherited_from for the predecessor.

Do not interpret a first-language fallback as detected English: unchecked/forced
labels have null confidence, and forced_route records the explicit override.
Guesses carry status, top_two, unrestricted_language and
outside_meeting_languages; observed_languages retains sub-window evidence.
M6 must account for that status and evidence, and honor the forced route. Thresholds
and thresholds_calibrated=false are part of the handoff. Calibration and the Hindi
route remain later work. No speculative skip flag bypasses those decisions.
