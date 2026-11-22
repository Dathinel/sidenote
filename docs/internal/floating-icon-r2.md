Decisions needed: none. The dev.20 follow-ups are implemented on `spike/floating-icon-r2`, source commit `285c651702344d2e663acbf6868d6318ecd5ea90`. All four mandated gates passed (R4). The isolated release run completed 50 show/card/teardown cycles and all functional probes (R5). The preserved strict cold-resource assertion still failed; it is not reported as a passed gate.

The branch was rebased onto `release/dev19`, pinned `5d607aa16708c7e13b201dc3e0c1a737ed48e78e`. `git merge-base` confirms that ancestor. Seven prior commits were replayed. The only manual conflict was Cargo.toml: retain dev.19's version and the spike's `default-run`. The pre-rebase tip is preserved locally as `backup/floating-icon-r2-before-dev19-20261112`.

R2 source changes: `RETIRING` holds at most two workers under its mutex. After a third teardown exceeds the existing two-second bound, the already-cloaked third worker detaches and `DISABLED` is set for the process lifetime. `ICON_DISABLED_STUCK_WORKERS` logs once. No further GUI workers start, even after earlier workers recover. At worst there are two retained stuck workers plus the third detached stuck worker; the cap is not a claim that only two threads can remain. Rust threads are never forcibly terminated. The monitor stays alive and normal listener detection/processing code is unchanged by this follow-up. Replay diagnostics write only numeric counts and a boolean to the isolated data directory.

There is one TODO immediately before `meeting_icon::start` at `src/listener.rs:436`, naming `ui_allowed()` and `feat/linux-server`. No device-role gate or call to `ui_allowed()` was added. The owner will wire it after that lane lands. The prior private drawing, unwind boundaries, nonactivating automatic show, transcript refresh and pending deadlines remain. The spike remains an example rather than a shipped binary.

R4 gates, stable Windows GNU target and shared target directory:

```powershell
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo clippy --locked --all-targets --features asr-runtime -- -D warnings
cargo build --locked --release --features asr-runtime
```

Clippy completed in 1m13s and 1m09s; the release build took 11m03s. No test suite ran. `src/main.rs` was touched before building. The initial copy used the wrong target directory and failed; it was immediately corrected to the GNU target subdirectory. The copied executable's timestamp matched this completed build, and the running cap diagnostics verified this implementation. The executable used for measurement is `artifacts/floating-icon-r2-dev20/sidenote.exe`, SHA-256 `C1BB3074C370EF9FF943E4ED619100D3C19F7C29223E394FBA3FF5DF609A6CB8`. It was copied before launch and never launched from the shared target directory.

R5 memory, MB = 1,000,000 bytes. Counts are object/handle counts. Three samples were taken three seconds apart at cold idle, shown, and teardown cycles 1, 10, 25 and 50. All three readings at each of these checkpoints were identical. Each other cycle has one observation 500 ms after both windows disappear. Cycle one additionally exercises scrolling, live refresh and close/reopen paths. No working-set trimming was used.

| Teardown cycle | Working set, MB | Private bytes, MB | GDI | USER | Handles |
|---|---:|---:|---:|---:|---:|
| 1 | 16.179 | 4.055 | 4 | 2 | 183 |
| 10 | 15.843 | 3.346 | 4 | 2 | 183 |
| 25 | 15.892 | 3.576 | 4 | 2 | 183 |
| 50 | 15.999 | 3.600 | 4 | 2 | 183 |

Cold idle: **12.517 / 2.531 MB** working set/private, GDI 0, USER 1, handles 170. Icon shown: **13.799 / 2.683 MB**, GDI 2, USER 4, handles 178. Each value repeated in all three samples. After the separate callback/timeout probes: **17.388 / 4.100 MB**, again identical across three samples; fault-path measurements are not mixed into the 50 normal-cycle series.

Memory reaches an **approximate plateau during this run**, rather than increasing once per meeting. From cycle 25 to 50 the endpoint changes are **+0.107 MB working set / +0.024 MB private**. Those 26 observations range from **15.790 to 16.503 MB working set** and **3.178 to 3.903 MB private**, with rises and falls. GDI remains **4** and USER **2** across every normal teardown; handles range **182–183** and equal 183 at all requested checkpoints. Every normal teardown observation enumerates zero owned windows. This supports a short-run plateau, not a strict long-run memory ceiling or absence of a small leak. Residual object identities remain a follow-up. The strict cold-baseline assertion remains false (cold 0/1 GDI/USER versus warm 4/2), so the measurement script deliberately exits **1** after saving successful functional results. The previous five-cycle failure is preserved in Git history.

R5 behavior evidence from this same executable:

- Drift: **0 px**, right edge **1910 px**, fixed **130 x 55 physical pixels**, over **20 bead/pill cycles and 1,000 geometry samples**. Empty bead space resolves through to underlying windows; icon/card display affinity remains `0x11`.
- Fifty normal cycles each show the icon, open the card without activation, verify native text/privacy behavior, acknowledge a synthetic draft, and tear down. The card has zero child controls, empty native text and no native accessibility provider. Close/hide does not acknowledge; destruction/reopen succeeds. Scrolling uses private semantic actions shared with the input handlers, without sending input.
- Live transcript paint changed with a minimum **1037 ms** refresh gap. The skipped notice was observed for **2.965 seconds** after detection. Missing-draft expiry was checked immediately before and at the ten-minute deadline using the isolated clock.
- Both synthetic callback panics were contained. A five-second stall exercised the two-second teardown bound; replacement appeared **189.107 ms** after the timeout log, with the previous icon cloaked and one visible replacement.
- Three overlapping replay-only stalls each exceed the join bound, with a 20-second safety deadline and an explicit release marker. Retained-worker counts were **1, 2, 2**. All three old windows were cloaked, the disable code appeared **once**, and no replacement appeared after the workers were released and reaped. The listener stayed alive; its monitor accepted and acknowledged another synthetic meeting while the overlay stayed disabled. This exercises the stuck-worker branch, not an infinite native hang.
- None of **1289 foreground observations** identified the owned process. Pending state finished empty. The owned process exited normally and its PID was checked absent afterward.

Committed evidence: `docs/internal/floating-icon-r2-evidence.json` contains all 50 cycle observations and all probe results. `docs/internal/floating-icon-r2-memory.html` plots the complete series. Full run artifacts, including the synthetic card render, are in `artifacts/floating-icon-r2-dev20/run-20261112-103345/`. Reproduce with `tools/measure-floating-icon-r2.ps1 -Exe artifacts/floating-icon-r2-dev20/sidenote.exe -Out artifacts/floating-icon-r2-dev20`.

R5 is confined to the owned isolated release process. Real Hex detection/processing, physical keyboard/click/drag behavior, dynamic DPI/monitor changes, and third-party capture behavior remain unproven. R2 confirms the listener's detection/processing code was left alone by the cap change. No input injection, focus-taking calls, installed-app/listener-task changes, pushes, GitHub operations or real meeting content were used. Owner steps needed for this run: none. Future owner follow-up: wire `ui_allowed()` after `feat/linux-server` lands.

Time buckets: orientation/rebase and editing completed before verification; optimized compilation dominated waiting at 11m03s. The failed copy command was the only rework and required no rebuild. No gate was bypassed or weakened.
