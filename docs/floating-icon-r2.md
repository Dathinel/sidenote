Decisions needed: none. The seven review fixes are implemented in `d871a7c608ab5bdbf1bc4c9c50ab08fd525c73e7`, on `spike/floating-icon-r2`, against review base `869904a9c679c426e230ded56525b8c38be6ed6c`. Mandated gates pass. All functional probes pass on the final release executable. Exact cold-resource restoration remains a recorded failure; retained object identities and long-run memory behavior are unproven.

The review changes, R2 source evidence:

1. `src/meeting_icon/native/window.rs` draws card text with GDI `DrawTextW` in its own `WM_PAINT`. There are no child controls. Meeting text never enters a native window-text property. The card returns zero for `WM_GETTEXT`, `WM_GETTEXTLENGTH`, `WM_GETOBJECT`, `WM_PRINT`, and `WM_PRINTCLIENT`. Hidden content is cleared. This closes the ordinary window-text disclosure; it does not protect against process-memory or filesystem access by the same user.
2. Handles and geometry use `Cell`; `RefCell` guards cover pure Rust state snapshots/updates only. No guard crosses `DefWindowProcW`, `DestroyWindow`, painting, show, resize, or other dispatching calls. Title-bar `SC_CLOSE` reaches `WM_CLOSE`, which hides and clears the card without acknowledging a draft. `WM_NCDESTROY` clears tracked handles. Explicit Enter acknowledges a displayed draft; Esc closes without acknowledgement. The existing main-window acknowledgement remains.
3. User clicks may activate the normal card for keyboard use. Automatic show uses `SW_SHOWNOACTIVATE`; position/resize uses `SWP_NOACTIVATE`; refresh invalidates its paint. The policy is commented next to the show call. The icon remains nonactivating.
4. Stopped short recordings show `Skipped` for a three-second policy interval, then retire. A stopped recording without a valid draft/final retires after **ten minutes**. This allows normal queued generation/retries without indefinitely retaining GUI resources when notes are disabled or unavailable. Notes jobs and recording files are not cancelled or deleted. Stop times persist across restarts; monotonic deadlines also bound a running process if the wall clock moves backward. The isolated replay clock accelerates only qualification.
5. Rolling-transcript revision travels with each frame. Visible transcript-only cards refresh when that revision changes, at most once per second. State/summary changes refresh separately.
6. Both product `extern "system"` procedures use `catch_unwind`; the callback panic hook suppresses payloads and logs only `FI_ICON_CALLBACK` or `FI_CARD_CALLBACK`. Failed hosts stop and restart through the monitor. Teardown requests shutdown within nested native dispatch, cloaks old windows through DWM, and waits **two seconds** on the native thread handle. Rust `join` is called only after native termination, including TLS cleanup. Timed-out workers stay tracked and cloaked; unique class generations permit replacement without two visible icons. Idle maintenance reaps exited workers. A cloak failure logs `FI_CLOAK_FAILED` and fails closed. Rust unwinding does not isolate native access violations or allocator aborts.
7. The standalone spike moved to `examples/meeting-icon-spike.rs`; its own FFI callback also has an unwind boundary. Cargo metadata reports kind `example`, not `bin`. Default shipped builds exclude it. Developer command: `cargo run --example meeting-icon-spike`.

R4 mandated gates passed, using the shared target directory and stable GNU toolchain:

```powershell
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo clippy --locked --all-targets --features asr-runtime -- -D warnings
cargo build --locked --release --features asr-runtime --bin sidenote
```

No test suite ran. `src/main.rs` was touched before the measurement build. The copied executable is `artifacts/floating-icon-r2-review/sidenote-final.exe`, SHA-256 `30DD6DC4B245B2997FD4EDFC200703A9400CBD0EFDEE01E39ABA3D12737B0A19`.

R5 below means the running isolated release listener, with synthetic files and ordinary listener workers, notes disabled, and Hex discovery redirected to an empty test directory. MB = 1,000,000 bytes; each row contains three samples, three seconds apart. No working-set trimming was used.

| State | Working set samples, MB | Private bytes samples, MB | GDI / USER, final sample |
|---|---|---|---|
| Cold idle | 12.050, 12.050, 12.050 | 2.396, 2.396, 2.396 | 0 / 1 |
| Icon shown | 13.582, 13.582, 13.582 | 2.560, 2.560, 2.560 | 2 / 4 |
| Teardown 1 | 15.757, 15.757, 15.757 | 3.391, 3.391, 3.391 | 4 / 2 |
| Teardown 2 | 15.258, 15.258, 15.258 | 2.863, 2.863, 2.863 | 4 / 2 |
| Teardown 3 | 15.270, 15.270, 15.254 | 2.863, 2.863, 2.830 | 4 / 2 |
| Teardown 4 | 15.503, 15.503, 15.503 | 3.076, 3.076, 3.076 | 4 / 2 |
| Teardown 5 | 15.540, 15.540, 15.540 | 3.138, 3.138, 3.138 | 4 / 2 |
| After resilience probes | 16.425, 16.425, 16.425 | 3.265, 3.265, 3.265 | 4 / 2 |

GDI and USER counts did not accumulate across five show/card/teardown cycles. Memory varied non-monotonically. The first-to-fifth settled samples decreased 0.217 MB working set and 0.253 MB private. Warmed cycle two to five increased **0.282 MB working set and 0.275 MB private**. The first cycle also exercises the longer live-refresh/scroll/close paths. These observations do not establish absence of a small long-run leak. Every enumerated native icon/card window was gone after each normal teardown. The retained four GDI objects and extra USER object are not identified as a proven cache or leak.

R5 functional results:

- Drift was **0 px**, right edge **1910 px**, fixed **130×55 physical pixels**, across **20 bead/pill cycles / 1,000 geometry samples**. The bead's empty region resolved through to underlying windows; both top-level capture affinities were `0x11`.
- Visible and hidden card text reads returned empty, text length and client accessibility request returned zero, and child enumeration returned zero. `SC_CLOSE` hid/cleared without a seen marker. Actual card destruction followed by reopening succeeded.
- Private replay actions exercised the same arrow/page/home/end/wheel scrolling logic, including clamping. No keyboard or mouse messages were injected. Actual physical key delivery remains untested.
- The rendered card changed as synthetic rolling text grew; the minimum measured live refresh gap was **1,072 ms**. The headless PNG uses the same drawing routine as `WM_PAINT`; no native capture exclusion was disabled.
- The skipped notice was observed for **2.536 s after detection by the probe** before teardown. The ten-minute missing-draft boundary was checked immediately before and at expiry using the isolated clock; production ten-minute waiting was not slept through.
- Synthetic panics in both callbacks logged their fixed codes, left the listener alive, and allowed replacement. A five-second GUI stall triggered the two-second retirement timeout. A replacement icon appeared **223.529 ms after the timeout was logged** while the old icon was cloaked; only one visible icon existed. The retiring worker subsequently exited and was reaped.
- None of **1,152 foreground observations** identified the owned process. No artifact process remains. Pending state was empty after all flows.

Reproduce with `tools/measure-floating-icon-r2.ps1 -Exe artifacts/floating-icon-r2-review/sidenote-final.exe`. Artifacts are under `artifacts/floating-icon-r2-review/run-20261112-092137/`. The committed `docs/floating-icon-r2-evidence.json` retains every sample and all functional results. The script still exits **1** for the pre-existing strict cold-baseline assertion; its functional-probe result is true. This is separate from the four passing mandated gates.

Unproven: physical keyboard/drag/click behavior, real Hex/speech integration, main-window-close interaction on the owner's screen, DPI/monitor changes, third-party capture behavior, long-run leak absence, and native-fault/allocator-abort isolation. The ten-minute production policy and activation-on-human-click behavior have R2 code evidence; neither was exercised by taking focus or injecting input. Owner steps required: none. No installed app/task operations, push, or GitHub operations occurred.

Time buckets: orientation/review, implementation, gate/probe verification, and compiler waiting. Release compilation dominated. The first link took 7m13s and was used for the first full behavior run; the final link took 9m06s. The largest repeated cost was that extra link to tighten the native thread-termination bound. No gate was bypassed or weakened.
