Decisions needed: none. Round 2 is committed and the mandated gates pass. Right-edge drift is fixed in the measured run. **Exact return to cold idle memory/resources remains a failed acceptance criterion.**

Branch `spike/floating-icon-r2`, based on the requested `fb9df8414e5ff3aeb388c684f5e2c4464cb50dab`. Implementation commits: `1f9c19a04b9c84905849629b7167877be2c980c4` and `64accf5c0e8f572c95869feedbd64e911388a8ca`.

The listener owns a native GDI icon instead of starting a Rune application/GPU surface. Its GUI thread, HWNDs, font, and drawing resources are created only for a live or unseen meeting. The thread ends after acknowledgement. A fixed 104×44-point transparent canvas contains the 44-point bead or 104-point pill. Its native region excludes the empty area. Speech appearance follows growth in the existing rolling transcript, with a two-second hold; it is not a microphone VAD signal. The read-only native card opens on click, never hover. Local draft/final content is acknowledged by the visible main meeting view or when its displayed card closes. Pending meetings and vertical position persist in the configured data directory.

Evidence R5 below means the running, isolated release listener with synthetic files, not the installed application. MB means 1,000,000 bytes. Samples are three seconds apart, without working-set trimming.

| State | Working set, three samples (MB) | Private bytes, three samples (MB) |
|---|---|---|
| Idle, no meeting | 11.006, 11.006, 11.006 | 2.351, 2.351, 2.351 |
| Icon shown, idle bead | 12.370, 12.370, 12.370 | 2.515, 2.515, 2.515 |
| After first teardown | 14.250, 14.230, 14.230 | 2.699, 2.662, 2.662 |
| After second meeting/teardown | 14.279, 14.279, 14.279 | 2.671, 2.671, 2.671 |

R5: idle icon CPU was 78.125 ms over 10.98948 seconds, **0.711% of one logical core**. Replay polls at 10 Hz to drive the cycles; production status polling is every 1.5 seconds. The regular listener workers also run, with notes disabled and Hex discovery redirected to an empty test directory. This does not establish connected-production CPU or memory.

R5: all **1,000 geometry samples across 20 bead/pill cycles** were 130×55 physical pixels, right edge **1910 px**, range **0 px**. The bead's empty area was outside its input region and `WindowFromPoint` resolved through it. Both windows reported capture affinity `0x11`. The card received its recording text. Closing it while recording preserved the icon; stopping and publishing an unseen draft preserved the icon; acknowledging the draft removed both windows. A second meeting recreated and removed them again. None of 1,115 foreground samples identified the owned process. All launched artifact processes exited.

The extra cold-resource probe deliberately exits **1**. USER count returns from 4 to its startup value of 1, but GDI count goes from 0 to 4 after the full flow and stays 4 after the second teardown. Window enumeration is empty afterward. The first settled teardown exceeds cold idle by **3.224 MB working set and 0.311 MB private bytes**. The retained GDI objects' cause is not established; they must not be described as a proven cache or a proven leak. All measured values are below the lead's 40 MB target, but zero idle-footprint growth is **not** established.

R4: these mandated commands passed on the final Rust source, using the shared target folder and stable GNU toolchain:

```powershell
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo clippy --locked --all-targets --features asr-runtime -- -D warnings
cargo build --locked --release --features asr-runtime --bin sidenote
```

`src/main.rs` was touched before the measurement build. The executable was copied to the worktree before launch. Its SHA-256 is `ADA35A9032D4DAE11A8990CDDA9CEB1976CFEF54A297350EDD52827E99EC65F6`. Reproduce with `tools/measure-floating-icon-r2.ps1 -Exe artifacts/floating-icon-r2/sidenote.exe`. The script retains all measurements before reporting its failed cold-resource check. The committed JSON contains every sample and that failure.

Artifacts are under `artifacts/floating-icon-r2/run-20261112-023722/`: `measurements.json`, `geometry.json`, `data/local/floating-icon-bead.png`, `data/local/floating-icon-pill.png`, and `floating-icon-card.png`. The icon PNGs use the actual offscreen drawing buffer. The card PNG is a headless preview of text read from the owned EDIT control, not a capture of native window pixels; capture remains excluded.

Unproven: exact cold-idle restoration, real Hex/speech integration, the actual main-window-close interaction, physical dragging, DPI/monitor changes, native card appearance and third-party screen-share behavior. Process ownership/lazy allocation are R2 code evidence; the main-window-close flow was not exercised on the owner's screen. No owner step was needed. No test suite, mouse/keyboard injection, foreground activation, installed-app/task operation, `hex use`, `hex model install`, or push was performed.

Time cost was dominated by release compilation: the cold build took 14m56s; the final GUI-thread change took 6m56s. Rework included an extra optimized link for acknowledgement handling and corrections to the native measurement script. No build or validation gate was bypassed or weakened.
