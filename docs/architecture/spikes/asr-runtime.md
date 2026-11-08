# Local speech runtime

Decisions needed: none. Use `ort` 2.0.0-rc.10 with `load-dynamic` for both measured ONNX packs. The optional backend builds with `--features asr-runtime`; the compatibility build loads no ONNX runtime.

## Choice and proof

The app remains `x86_64-pc-windows-gnu`, static CRT and `rust-lld`. ONNX Runtime's C ABI needs no MSVC import library in the Rust link. The existing sherpa v2.0.1 package has its CLI and ONNX Runtime, but no sherpa C API DLL. M0b measured that CLI for Hindi; `lid/anuvani-eval.py` instead used Python ONNX Runtime. Stock sherpa support for Anuvani was not proved. The selected GNU Rust spike runs both graphs through one unmodified runtime, with no sherpa fork.

`Anuvani::load(&Path)` and `IndicConformer::load(&Path, &Path)` retain sessions across the meeting. CPU inference uses one thread and windows of at most 24 seconds. The Hindi frontend uses Rust `kaldi-fbank`, NeMo normalization and CTC decoding. ASR confidence stays unknown. LID returns conditional English/Hindi scores plus all 23 unrestricted scores; low margins never mean mixed speech.

A Windows loader preflight checks DLL dependencies and C API version 22 before ort's panic-based loader. This makes unavailable or incompatible runtime files ordinary errors even with release panic-abort. File-open and session-load errors propagate to the router's marked English fallback. The runtime is pinned for the process lifetime.

## Measurements, evidence rung 4

On M0b's same first 50 public Hindi clips, the GNU spike scored 11.70 Latin ITRANS/jiwer WER versus 12.8. Forty outputs exactly match sherpa's earlier outputs. The remaining differences were not isolated; the 1.1-point aggregate difference is within the three-point tolerance. This does not prove parity on other languages.

Anuvani loaded in 1.547 s; Hindi loaded in 0.814 s. Combined inference took 84.15 s, total process wall time 86.86 s, sampled peak working set 625.6 MiB. Both sessions stayed loaded across all 50 clips. The earlier 50-stream sherpa batch reached 8,683 MiB; bounded sequential windows avoid its padded batch allocation. These are different methods under load, not controlled speed comparisons.

Verified dev executable sizes are 164,361,216 bytes without the runtime and 179,379,712 with it: 15,018,496 added bytes. Release installer size and clean-machine deployment were not measured. Stable fmt, locked clippy with all targets and warnings denied, and locked builds pass with and without the feature. No test suite runs. Refresh both `src/main.rs` and `src/lib.rs` before a shared-target build, then copy the executable before another lane replaces it.

## Fetch and catalogue integration

Ship `onnxruntime.dll` (17,136,128 bytes) and retain `onnxruntime_providers_shared.dll` (10,752 bytes) from the [sherpa v2.0.1 Windows x64 shared MD no-TTS release](https://github.com/k2-fsa/sherpa-onnx/releases/tag/v2.0.1). Omitting the providers DLL was not verified. Import inspection also found `MSVCP140.dll`, `MSVCP140_1.dll`, `VCRUNTIME140.dll` and `VCRUNTIME140_1.dll`; Microsoft's x64 C++ redistributable remains a prerequisite despite the app's static CRT. No installer wiring is included.

Get Hindi from the [measured MIT sherpa pack](https://huggingface.co/parismitaglobalsolutions/indicconformer-sherpa-onnx) and Anuvani from [CIIL, Apache-2.0](https://huggingface.co/CIIL/Anuvani-LID). After rebasing onto the catalogue merge (`cbc6a5a`), `asr::model_path` uses the real installed-path snapshot for `indicconformer-hi-ctc` and `anuvani-lid-int8`. An optional state-directory argument supports isolated proof. Constructors still reopen each graph/token file; catalogue lookup holds no inference lease. Downloads and SHA256 verification remain explicit catalogue actions. No model, DLL, audio or transcript belongs in git.

Reproduce native execution with `cargo build --locked --features asr-runtime --example asr_spike`, then `asr_spike.exe <absolute DLL> <Anuvani graph> <Hindi graph> <tokens.txt> <public WAV or folder>`. Full routing numbers are in the child branch's `meeting-router-results.md`. Live meeting quality and clean-machine packaging remain unverified.

DLL integrity: pin SHA256 422d776ab0e3218260f7f628fcb84606aaa5c21116f720c8619a6da5e0b2e0f9 for the documented 17,136,128-byte DLL. Before every initialization, hash an open handle that denies writes and deletion through loading. A different build is unavailable. This does not contain inference aborts; the jobs layer must supervise the inference process and publish fallback on a crash.
