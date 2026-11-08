# Routed transcript UI evidence

Decisions needed: none. This branch adds presentation to the runtime, core and jobs stack. One Languages-tab control changes language handling; the catalogue's saved declarations remain the only language preference. Help calls the mixed route "the mixed Hindi-English model". It does not name a model in that help.

Ordinary English lines have no route caption. Hindi/mixed routes and marked English fallbacks keep their captions. Group timestamps refer to the real group interval, not guessed word positions. Copy uses the displayed view; note citations use the original English index space.

Meeting loading computes a routed-content revision. The catalogue comparison checks that revision alongside the existing English transcript and notes, so an open meeting receives a new routed artifact even when its other text is unchanged. The comparison itself only compares strings. The headless Original scene first opens a meeting, then applies a real `Event::Meetings` changing only its routed content/revision. The rendered Jordan line says "Prepare the updated color samples." This exercises the previously missed refresh path without private audio or the installed app.

The final UI dev executables are 166,298,112 bytes without the backend and 181,500,928 with it. The feature executable SHA256 is `4df318ad9454ad543ef17daa98ab44e49c50ad6c2c8f7769e6cf810f0cac0aaf`. Stable fmt, locked clippy with all targets and warnings denied, and locked builds pass in both configurations (`reviews/sidenote-router-build/router-ui-final-gate-*`). No tests ran. The copied feature executable produced all eight PNGs below; main and lib were refreshed before the build.

Renders under `E:/Personal-Projects/hex-meetings/.worktrees/router-ui/artifacts/router-ui/`:

| Scene | Dark | Light |
|---|---|---|
| Languages, wide | `routing-settings-dark.png` | `routing-settings-light.png` |
| Languages, narrow | `routing-settings-narrow-dark.png` | `routing-settings-narrow-light.png` |
| Original, wide | `routing-original-dark.png` | `routing-original-light.png` |
| Original, narrow | `routing-original-narrow-dark.png` | `routing-original-narrow-light.png` |

These are synthetic meeting renders using isolated state. The installed app's filesystem watcher and private meeting audio were not exercised. Per-turn overrides/reruns and word alignment remain explicit follow-ups outside this round. The new processing path stays off by default. Evidence rung 4 applies to the headless refresh event; the inspected PNGs demonstrate the rendered UI only.
