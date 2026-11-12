Decisions needed: none.

# Experimental prepared-chunk routing

The external pipeline now connects preparation to the standalone M5 CLI before
transcription. No installed settings are discovered. Supply an explicit snapshot
with `labs.language_detection=true` through `-RoutingConfig`; the existing Labs
switch defaults false. `-LanguageDetectExe` can select an undeployed M5 binary.
With no snapshot, or a false switch, the transcription calls and artifact format
stay on the previous path. The ASR configuration/cache key is unchanged.

Observe adapts each padded PCM16 WAV into bounded M5 windows without resampling or
cutting away padding. Window offsets are relative to that WAV; the adapter records
the original chunk ID, track, speaker and meeting/audio bounds alongside the PCM
hash. Windows from different prepared chunks never inherit a short label.
Missing, stale, incomplete or failed evidence runs both Telugu and Hindi models.
Any evidence observing multiple languages also runs both models, regardless of
calibration or English coverage.

`-RoutingMode shadow` is the default and still calls both models. `execute` checks
the saved plan again against current PCM, settings, requested model IDs and English
coverage. English coverage requires nonempty readings over the full speech interval
on the same track. Selected calls retain model grouping and empty merge fields.
An empty selected reading retries the omitted model. Metadata records each route,
reason, fallback count, detector hash and requested ASR model IDs; engine versions
are marked `unreported` because the Hex API does not provide a pinned version here.
No transcript text is added to routing logs.

M5 currently always writes `thresholds_calibrated=false`. Automatic routing therefore
continues to run both models even in execute mode. There is no qualification bypass
flag. Explicit `pipeline.language_mode="forced:te"`, `forced:hi`, `forced:en`,
`forced:mixed` (English/Hindi), or declared `+` combinations can select calls.
Forced English still requires coverage. A wrong explicit override can reduce quality;
retain original recordings and use the all-model recovery path.

For a whole meeting, use the wrapper's `-AllModels`. For retained prepared chunks:
`meeting-audio.py transcribe --temp <prepared-directory> --all-models` reruns both
models; `--rerun-chunk <id>` reruns both for just that chunk and preserves the other
readings. Repeat the flag for several chunks. The wrapper's temporary directory is
removed on completion, so a later full wrapper rerun prepares from originals again.

Estimates from the supplied speedup plan: observe and shadow planning save zero
seconds and add unmeasured overhead. Selected execution could save 1,142 seconds
gross for the assumed 50% English / 25% Telugu / 25% Hindi mix and equal chunk cost.
The older detector rate suggests 361 seconds net for 64 minutes of continuously
analyzed audio. These are estimates, not measurements of this implementation.
Fallbacks, preparation, notes and startup costs still apply. Current uncalibrated
automatic execution saves zero ASR time.

Synthetic proofs run on boxes.dev without ASR or inference. Public FLEURS recognition
quality, natural code-switch recall, detector memory and runtime overhead remain
unqualified. The Windows-only M5 runtime cannot be qualified on the Linux test box;
the laptop is excluded from inference and test execution.
