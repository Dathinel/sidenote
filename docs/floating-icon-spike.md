Decisions needed: none. This spike supports the two-window interaction, but does not meet the separate-process memory target or the strict native-resize visual requirement for the selected hybrid bead.

# Floating icon Windows spike, 12 November 2026

The probe is a separate executable, `meeting-icon-spike.exe`, on `spike/floating-icon`, based on release/dev17 (base `f30e3a84156ba4c4e9361fbaa6621a2f5e1572f7`). It does not initialize Sidenote services or connect to Hex. Rune remains pinned at `a138b97e654b8299b2a4cb518b8476b2b528c2c4`; its checkout was verified with `git rev-parse HEAD`. No dependency or lockfile change was needed. The manifest sets `default-run = "sidenote"` to preserve the normal launch command after adding this binary.

Evidence rungs used here: R2 = inspected the pinned source; R4 = executed a check or unit test; R5 = observed the running Windows process. A reasoned result is explicitly distinguished from a live display-setting test.

| # | Requirement | Result | Evidence |
|---|---|---|---|
| 1 | Borderless, topmost, nonactivating right-edge overlay; 48 px bead and approximately 136×38 pill | PASS, R5 | At the owner's 125% scale, native bounds were 60×60 and 170×48 physical pixels. Both had a client area equal to the outer area, TOPMOST/TOOLWINDOW/NOACTIVATE styles, and unchanged foreground on show. The right edge was 1910, leaving an 8-point inset from the 1920-pixel work area. Rune reported successful DX12 presentation. |
| 2 | Vertical mouse drag, keys where possible, saved position | PASS with keyboard limitation, R5/R2 | Both shapes moved 96 physical pixels down and back up with real pointer input. The focused panel's Move control moved the icon another 20 physical pixels, equal to 16 points. The position file survived process restarts. Windows overlay keys are reported Unsupported; arrow controls therefore live in the normal panel. The updated proof also checks that an open card hides while dragging and reopens beside the dropped icon. |
| 3 | Separate approximately 360 px normal borderless topmost panel; working text input; close leaves icon alive | PASS, R5 | Native panel bounds were 450×325 physical pixels, equal to 360×260 points. It took foreground focus, accepted `spike keyboard 123`, closed independently, and reopened with its text retained. |
| 4 | Both windows excluded from capture; confirm the API call | PASS for the Windows API contract, R5 | Normal runs returned `GetWindowDisplayAffinity = 0x11` for both windows. The normal panel logged `SetWindowDisplayAffinity=1`; the overlay reported capture exclusion Requested through Rune. Tray-opened panels also returned `0x11`. This is not a claim about every third-party recorder. |
| 5 | DPI at 100%/150% and monitor changes | PASS by code reasoning and focused test; live coverage limited | The live environment was one 1920×1080 primary monitor at 120 DPI/125%, with `rcWork=(0,0,1920,1080)`. Owner settings were left untouched. The test covers 100%/150%, both original heights, clamping, negative origins, and a too-small work area. The process polls primary-monitor identity, DPI and full work rectangle every 500 ms and reapplies geometry after a change. Physical unplug/replug and actual 100%/150% rendering remain untested. |
| 6 | Icon process idle working set below 40 MB | FAIL, R5 | Final tray-enabled process: bead 127.32 MB, pill 128.26 MB, hybrid 126.63 MB, restart 126.98 MB. These are unoptimized debug builds, including the precreated hidden panel and tray, measured with `Get-Process` after ten idle seconds. No release-memory claim is made. |
| 7 | Selected hybrid: runtime width 44→104→44, anchored without flicker; CPU and appearance | FAIL for strict visual requirement; anchoring workaround passes, R5 | At 125%, width traversed 55→130→55 physical pixels. Public Rune resize/place calls transiently moved the right edge between 1897 and 1931, versus intended 1910. A Win32 `WM_WINDOWPOSCHANGING` subclass fixed all 924 sampled right edges at 1910. `Tx::instant()` reduced the 20.04-second sequence to 984.375 ms of CPU, 4.91% of one core, versus 0.72% idle. Native resize still produced brief shape/edge stretching: the 60 fps recording flagged 15 of 1740 frames in a fixed painted-cap region. There was no hidden-window sample. |
| 8 | System tray item “Show meeting panel” | PASS, R5 | Rune already supplies the API. Shell registration was confirmed by `Shell_NotifyIconGetRect` returning success. The actual native popup contained the label; selecting it invoked `TrayEvent::Menu("show-panel")`, opened the excluded keyboard-capable panel, and the Quit menu exited the process and removed the registered tray icon. |

The implementation is [the standalone binary](/E:/Personal-Projects/hex-meetings/.worktrees/floating-icon/src/bin/meeting-icon-spike.rs). Native measurements and probe outputs are preserved in [the evidence JSON](/E:/Personal-Projects/hex-meetings/.worktrees/floating-icon/docs/floating-icon-spike-evidence.json). The two original shapes remain available; `--hybrid` selects the new 44-point bead and 104-point pill. This is placeholder UI and synthetic resize timing, without speech detection or meeting controls.

**Exact framework limits and workarounds**

- Overlay keyboard requests have implementations only for macOS and Linux layer-shell at [Rune overlay/input.rs:407](/C:/Users/harsha_befach/.cargo/git/checkouts/rune-c9b805e9ed8a3730/a138b97/crates/rune/src/overlay/input.rs:407). Windows deliberately uses NOACTIVATE at [platform/win32.rs:226](/C:/Users/harsha_befach/.cargo/git/checkouts/rune-c9b805e9ed8a3730/a138b97/crates/rune/src/platform/win32.rs:226). The normal panel and tray command provide the keyboard path. No pointer-drag patch to Rune was needed.
- Rune receives Windows pointer movement and left-button events at [overlay/any_host.rs:111](/C:/Users/harsha_befach/.cargo/git/checkouts/rune-c9b805e9ed8a3730/a138b97/crates/rune/src/overlay/any_host.rs:111). [overlay.rs:912](/C:/Users/harsha_befach/.cargo/git/checkouts/rune-c9b805e9ed8a3730/a138b97/crates/rune/src/overlay.rs:912) compensates drag coordinates when its own window moves. Live Down/Move/Up logs confirmed this path. The automated physical-input proof allows 100 ms for the press to settle before motion; it is not an adversarial input-timing test.
- The Windows work-area helper returns only top and bottom at [platform/win32.rs:256](/C:/Users/harsha_befach/.cargo/git/checkouts/rune-c9b805e9ed8a3730/a138b97/crates/rune/src/platform/win32.rs:256). Horizontal edge placement uses full monitor width at [overlay/placement.rs:22](/C:/Users/harsha_befach/.cargo/git/checkouts/rune-c9b805e9ed8a3730/a138b97/crates/rune/src/overlay/placement.rs:22). The spike obtains the complete `rcWork` and adds the right-side reservation to the inset. A right-side taskbar was not exercised live because this monitor reserved no taskbar strip in `rcWork`.
- [windows.rs:189](/C:/Users/harsha_befach/.cargo/git/checkouts/rune-c9b805e9ed8a3730/a138b97/crates/rune/src/windows.rs:189) is a `MacWindowOptions` capture setting, not a Windows window option. The spike calls `SetWindowDisplayAffinity` in the normal window's creation callback. Rune already calls the API for overlays at [platform/win32.rs:110](/C:/Users/harsha_befach/.cargo/git/checkouts/rune-c9b805e9ed8a3730/a138b97/crates/rune/src/platform/win32.rs:110).
- Native size and position are separate requests at [overlay.rs:991](/C:/Users/harsha_befach/.cargo/git/checkouts/rune-c9b805e9ed8a3730/a138b97/crates/rune/src/overlay.rs:991) and [overlay.rs:951](/C:/Users/harsha_befach/.cargo/git/checkouts/rune-c9b805e9ed8a3730/a138b97/crates/rune/src/overlay.rs:951). The optional `--atomic-anchor` subclass adjusts each `WINDOWPOS` before Windows applies it. This corrects geometry, but does not make the new GPU image appear atomically with native resizing. Attribution of the observed stretching to that repaint gap is an inference from these calls and the recorded frames, not a DWM trace.
- Rune's existing [tray.rs:91](/C:/Users/harsha_befach/.cargo/git/checkouts/rune-c9b805e9ed8a3730/a138b97/crates/rune/src/tray.rs:91) accepts a menu and callback. Its [Windows backend](/C:/Users/harsha_befach/.cargo/git/checkouts/rune-c9b805e9ed8a3730/a138b97/crates/rune/src/tray/win32.rs:118) registers with `Shell_NotifyIcon`. No new tray library or dependency was added.

Winit keeps WS_CAPTION in its native style even for undecorated windows and removes the non-client area through `WM_NCCALCSIZE` (`winit-0.30.13/src/platform_impl/windows/event_loop.rs:1162`). Borderlessness was checked with client-to-screen bounds, not inferred from the caption bit.

The predicted sizes at the requested scales follow the same logical-to-physical conversion that produced the observed 125% dimensions:

| Scale | Original bead | Original pill | Hybrid resting / grown | Placeholder card |
|---|---|---|---|---|
| 100%, reasoned | 48×48 | 136×38 | 44×44 / 104×44 | 360×260 |
| 150%, reasoned | 72×72 | 204×57 | 66×66 / 156×66 | 540×390 |

All sizes in that table are physical pixels. Local offsets are scaled; desktop origins remain physical and may be negative. Saved position is a fraction of the available vertical travel, so it is recomputed against the new work area rather than restoring an obsolete pixel coordinate. On a monitor/work-area change the icon is placed again and an open card is clamped and repositioned without reopening it. A transient monitor-query failure leaves the last geometry in place and retries on the next poll. Rune handles a native scale-factor event in `overlay/any_host.rs:98`. This is code reasoning, not a hot-plug reproduction.

**Resize measurements**

The synthetic workload performs ten grow/shrink cycles. Growth requests seven width steps over approximately 220 ms; shrink requests six over 180 ms. Each endpoint rests for 800 ms. CPU percentages below are fractions of one logical processor; this machine reported eight logical processors. The corrected 4.91% core result is approximately 0.61% of total eight-processor capacity. It is an average across transitions and endpoint rests, not a peak animation cost.

| Run | Geometry samples | Right-edge range, physical px | CPU during resize sequence | CPU while idle | Presents / failed acquisitions |
|---|---:|---|---|---|---|
| Public Rune resize/place | 821 | 1897–1931 | 2968.75 ms / 20.108 s = 14.76% core | 62.5 ms / 5.594 s = 1.12% core | 930 / 0 |
| Native anchor + instant Rune transactions | 924 | 1910 only | 984.375 ms / 20.044 s = 4.91% core | 46.875 ms / 6.550 s = 0.72% core | 132 / 0 |

Appearance was checked separately from CPU measurement. A bounded 130×55-pixel, 60 fps lossless recording contains only the maximum prototype area and its transparent surroundings. `--visual-proof` disables exclusion for the synthetic overlay in that recording run; the normal panel stays excluded. Normal runs and CPU benchmarks keep affinity `0x11`. This test flag does not change any stored setting or another app's capture policy.

The recording is [hybrid-resize.mkv](/E:/Personal-Projects/hex-meetings/.worktrees/floating-icon/artifacts/floating-icon-spike/hybrid-resize.mkv). Representative transition frames are [resize-transient.png](/E:/Personal-Projects/hex-meetings/.worktrees/floating-icon/artifacts/floating-icon-spike/resize-transient.png). The paint probe compares a fixed 4×4 area inside the right cap, away from the changing label. Fifteen frames changed by more than eight RGB levels, with a maximum channel deviation of 31. The enlarged frames show transient stretching. This detects a visual defect; it is not a claim that every full-window flash or every transition artifact was counted. These private recordings are not committed.

**Reproduction and checks**

Run from `E:\Personal-Projects\hex-meetings\.worktrees\floating-icon`, with `E:\Personal-Projects\tools\hex-meetings-gnu\w64devkit\bin` first on PATH. The toolchain used was rustc 1.99.1 / cargo 1.99.1, GNU Windows target. The OS reported Windows build 26300; Rune selected DX12 on NVIDIA GeForce RTX 3050 6GB Laptop GPU.

```powershell
$env:PATH = 'E:\Personal-Projects\tools\hex-meetings-gnu\w64devkit\bin;' + $env:PATH
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --locked --bin meeting-icon-spike
cargo build --locked
```

All four commands passed on the final Rust source and manifest. The focused test reported one passed test and no failures. Formatting, Clippy and the locked build were rerun after the tray, resize and manifest changes.

The native scripts operate only on process IDs they launch. Physical clicks check the window under the pointer; keyboard injection checks the foreground and held modifiers. The tray proof targets only the probe's own native popup queue. It does not click Explorer or another application's windows.

The first successful physical-input run verified the original shapes, normal-window typing, arrow control, close/reopen and saved-position restart. A later replay was blocked because `WindowFromPoint` returned a `PickerHost` process instead of the spike. The guard refused input and that window was not dismissed. The final four runs therefore sent mouse messages directly to owned HWNDs and checked native geometry and visibility. All four passed card-hide-on-drag, reopen-on-drop, clamping, persistence, capture affinity, timed process exit and tray unregistration. The final hybrid measured 55×55 physical pixels; its drag moved from y=604 to y=700. The last bead run dropped at y=792 while its panel correctly clamped to y=755.

```powershell
# Original shapes, drag/card lifecycle, input, persistence, memory and tray cleanup.
& tools/floating-icon-proof.ps1
# Native tray popup Show and Quit, capture affinity and unregistration.
& tools/floating-icon-tray-proof.ps1
# Final owned-HWND event replay when another process covers the pointer.
& tools/floating-icon-window-message-proof.ps1
# Intentionally fails the strict right-edge assertion on unmodified Rune placement.
& tools/floating-icon-resize-proof.ps1
# Passes geometry and measures CPU with the small Windows workaround.
& tools/floating-icon-resize-proof.ps1 -AtomicAnchor
# Separate bounded recording of synthetic placeholders.
& tools/floating-icon-resize-proof.ps1 -AtomicAnchor -Visual
# Intentionally reports failure for the observed transition paint defect.
python tools/analyze-floating-icon-video.py
```

For a brief manual run, use `target\x86_64-pc-windows-gnu\debug\meeting-icon-spike.exe --hybrid --atomic-anchor --seconds 60`. Click the bead or use its tray menu to open the card. Quit through the panel or tray. Its default position file is `artifacts/floating-icon-spike/position.txt`, separate from all Sidenote and Hex settings.

**Recommendation for the real build**

Use the Rune overlay plus an ordinary Rune card, with Rune's tray API and explicit Windows capture exclusion for the card. Keep the card hidden during icon drag and position it once on drop. Retain shared state in the existing Sidenote process, including a deliberate keep-running/hide lifecycle, before claiming that closing the main window preserves the icon.

Keep the selected hybrid design, but investigate growing its contents inside a fixed 104-point native window instead of repeatedly resizing the native surface. Use pointer regions for the visible shape so the unused transparent area passes clicks through. Windows click-through and this fixed-canvas visual path still need a focused proof; they were not implemented here. If that fails, the already-proven static 44-point bead is the fallback described in round two. The atomic-anchor subclass is useful evidence, not a complete solution to the visual defect.

Do not assume a separate Rune process meets the 40 MB budget. The measured debug process does not. Release profiling and the incremental memory cost inside Sidenote remain open. A hard separate-process 40 MB budget may require a small native icon host; that alternative has not been measured in this spike.

Final cleanup was checked with `Get-Process -Name meeting-icon-spike`: zero processes remained. Each final run exited through its timer and the old tray identifier no longer resolved through the Shell API. The separate tray test also verified the explicit Quit menu path. No command to start or stop a Hex meeting, change Hex settings, open a browser, or read credential files was issued. Nothing was pushed.
