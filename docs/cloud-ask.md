# Ask chat completions (phase 1)

Settings > Models and agents > Ask connection accepts an OpenAI-compatible base
URL, model ID and optional API key. Save the connection, then select **Use this
connection for Ask**. Remote endpoints additionally require **Opt Ask in for this
endpoint and model**. Setup and model-list loading never send meeting content.

OpenRouter and Local presets only fill fields. Local presets cover Ollama
(`127.0.0.1:11434/v1`), LM Studio (`:1234/v1`), llama.cpp (`:8080/v1`) and vLLM
(`:8000/v1`). Replace the suggested model ID with the ID loaded by your server.
The OpenRouter selector starts with suggestions; **Load OpenRouter model list**
explicitly fetches its public `/models` catalogue without a key or meeting data.
Availability is established by the selected server, not by the preset.

The existing single-meeting, cross-meeting and live Ask prompt builders feed the
same direct streaming adapter. It has no tools or filesystem access. The request
uses a separate system policy treating recorded content as untrusted evidence.
This prompt policy is not a proof against misleading model answers.

The Ask label shows Local/Cloud, base URL and model; the completed attempt shows
first-token and total wall time, returned token usage, and **cost unknown**. The
15-second slow notice and failures offer Retry, Use local, or Pick another model.
Retry starts a new attempt. The other choices open settings for an explicit
selection and save; there is no automatic provider or CLI fallback. Existing CLI
Ask remains selectable with **Use installed CLI for Ask**.

Remote connections require HTTPS; plain HTTP is restricted to literal loopback
IPs and `localhost`. Redirects and environment proxies are disabled. A localhost
gateway can itself forward content; the UI discloses that limitation. Consent is
Ask-only and bound to the exact completion URL and model. Changes require fresh
consent. Revocation or connection changes stop receiving the current attempt;
already-transmitted content cannot be recalled. Cancellation returns promptly;
the HTTP worker releases a blocked socket on its bounded read/global timeout
(30/120 seconds) or when the server next responds.

On Windows, the existing `sidenote-sync::identity` DPAPI helper protects the key
for the current user. Ciphertext is stored separately from settings and bound to
the endpoint URL. The UI uses Rune's secure input (masked rendering, redacted
debug/accessibility/snapshots), supports replacement and deletion, and never
reloads plaintext into a field. The HTTP authorization value is marked sensitive;
transport errors use a fixed vocabulary and never include response/error bodies.
The decrypted buffer is zeroized by the existing helper. No new crates are used;
the existing ureq/rustls and DPAPI dependencies are retained unchanged.

Device settings/consent and ciphertext live in
`%LOCALAPPDATA%/sidenote/cloud-ask`, outside the synced state. An explicit data-dir
override isolates them under `<data-dir>/local/cloud-ask` for synthetic proofs.
Server and Hub endpoint execution is unavailable in this phase; their existing
headless startup path remains intact. Linux remote Ask reports secure-store
unavailability; an unlocked credential-store backend is deferred.

## Qualification

The new unit tests cover request shape/policy, URL validation and recipient-bound
consent, SSE split bytes/UTF-8/CRLF/usage/[DONE], malformed or tool-call events,
fixed/redacted errors, and refusal to enter the existing CLI fallback. The lead
runs tests in the release gate; implementation does not execute a test suite.

`cargo run --locked --example cloud_ask_bench -- 20` runs only an ephemeral mock
streaming server on `127.0.0.1`, with isolated temporary state. On Windows, half the requests use a synthetic
key to exercise DPAPI save/load/delete and the actual authorization header;
the other half exercise the optional-key path. It
asserts the actual HTTP request, fragmented SSE response and usage, and prints
per-attempt first-visible-text/total milliseconds with first-text p50/p95. These
numbers describe the loopback mock, not any cloud model's performance.

`python tools/render_settings_build.py --cloud-ask` writes dark synthetic Settings
and Ask-label PNGs and element trees into `artifacts/settings-build`, without
desktop interaction. The normal full render gate and settings-reachability gate
continue to cover the legacy controls.

Phase 2 adds separate notes/daily-check provider selections and consent,
schema-validated structured output, and paired synthetic quality/latency
qualification. The bounded read-only tool agent, memory proposals and server
credential provisioning remain later phases.
