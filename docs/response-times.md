Decisions needed: none.

Response timing is a session-local, opt-in diagnostic. Launch the worktree build
with `--response-times` to include startup, or enable it under Settings >
Diagnostics > Response times for subsequent actions. Disabling clears samples;
Clear starts a fresh recording window if recording is on. Nothing is persisted.

The producer runs only on the UI thread. The disabled path checks a thread-local
option. Enabled hooks read the monotonic clock and publish fixed-size numeric
records into a preallocated 2,048-slot single-producer/single-consumer queue.
They do not allocate, acquire locks, format text, write files, or send network
requests. The worker correlates operation IDs and calculates percentiles. Reading
Diagnostics uses a nonblocking snapshot; Refresh requests another view update.
The cost of the timestamp and atomic publication still needs enabled/disabled
measurement on the reference host. No zero-overhead claim is made.

Only fixed action tags, session-local operation numbers and monotonic durations
enter the queue. No hook accepts a meeting ID, path, transcript, prompt, answer,
model name, or other content. An operation of the same action supersedes an older
pending sample. Failures and supersessions are separate from successful samples.
Dropped records are counted. A recording with drops is incomplete and must not
be used to qualify a latency budget.

First is the first drawn frame after the handler starts. Ready is the first drawn
frame after the action has completed its state changes. Synchronous handlers
often have equal first and ready times. Search starts at its changed callback;
it does not include earlier OS dispatch or text-edit processing. The Tasks board
waits for the latest accepted Areas refresh, including refresh retries.
Repeated navigation to an already selected destination is excluded. Pane changes
supersede pending samples that no longer match the drawn view. Ask additionally
matches the visible thread's session-local operation number.

The pinned Rune callback runs on the UI thread after a frame is submitted and
presented to the graphics API. `FrameEncoded::at()` is stamped after encoding and
before presentation. It is an encoded-frame proxy, not proof of pixels appearing
on the physical display. Skipped and unchanged frames do not complete samples.
Background work, a hidden window, and a superseded navigation can delay or prevent
a usable sample. Metrics are diagnostic observations, not release qualification.

Diagnostics shows successful counts and first/ready p50/p95 in milliseconds over
the latest 256 successful samples per action. Counts, failures and supersessions
cover the recording session. CLI and connected Ask requests use separate fixed
categories based on the configured submission route; fallback classification and
warm/cold separation remain follow-ups. Stop measures the accepted in-app
Stop action through the next drawn frame after that recording's summary is
loaded. Stops made outside this app are not sampled.
The pending Stop belongs to its live recording instance. Replacing that instance
supersedes its sample; Stop replies must match its session-local identity.

The initial stack intentionally leaves persistent NDJSON export, deterministic
desktop replay, scroll gesture completion, stage-level pipeline timings and
calibrated release budgets for follow-up. Headless scene rendering does not call
the desktop frame hook and cannot validate presentation. Do not use the existing
real-meeting launch probe to qualify this feature.
The isolated `response-time-large` scene supplies 2,000 synthetic meetings and a
10,000-line transcript. The `response-time-search` render target scripts search,
meeting selection and finder input; this is a layout replay, not timing proof.

Validation on this laptop is limited to formatting, default/ASR/server Clippy,
build and test compilation with `--no-run`. Tests, benchmarks, app launches,
installed data, listener operations and GitHub writes are outside this run.
