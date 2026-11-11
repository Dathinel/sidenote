Decisions needed: none

# Annotation notebooks and sharing, round 1

Worktree: `E:/Personal-Projects/hex-meetings/.worktrees/annotations-notebook`, branch `feat/annotations-notebook`. No push or version bump. Builds use this worktree's own target folder. Version remains `0.0.1-dev.12`.

## Image scope and repeated screens

The section-to-capture mapping was correct: assembly copies figures from each capture's own note, and export resolves those figures against that capture's own approval. The synthetic captures reuse the same two PNGs, which caused the repeated screens and callouts in the original proof.

The app and HTML now use one reading projection. It hashes the approved marked PNG bytes, displays each hash once in the first section that uses it, and adds references such as "Same screen as Workshop setup, marks 1, 2." in later sections. Distinct frames receive notebook-wide callout numbers. If a later capture has edited callout wording or a different caption, that text remains beside its shared-screen reference.

Notebook numbers are drawn over the immutable approved PNG at display time. Export embeds the exact approved PNG bytes and uses an inline SVG for any renumbered badge. Stored capture metadata, approved images and scrawl bundles are unchanged. Reordering recomputes the first section and references. The synthetic notebook now displays two unique images and three callouts, rather than six images and nine repeated callouts.

Tests cover two captures sharing frames, a distinct frame receiving mark 4 while retaining its approved PNG hash, edited later callouts, and first-section references after reorder.

## Rows and controls

Notebook cards are compact 76 px rows: number, Lucide grip-vertical handle at 18 px and stroke 2.25, approved thumbnail or neutral placeholder, title, date/mark count/readiness, then Open and Remove. The handle tooltip is "Drag to reorder". Drag and Alt+Up/Alt+Down remain available. Capture rows use the same 72 x 44 px approved thumbnails. Thumbnail reads run in the background and verify approval hashes.

The single muted instruction is: "Uses each capture's note, no images sent. Drag or Alt+Up/Down to reorder."

## Live timings

Three successful live calls out of the six-call limit. Each used the same three synthetic captures and existing per-capture notes; zero images sent. Timing starts before the CLI call, first text means text visible through the app's stream parser, and total includes note validation. Model defaults were not changed. Haiku used the CLI alias `haiku` at Low effort.

| Agent / model | Effort | First visible text | Complete valid note |
|---|---|---:|---:|
| Claude Code / claude-sonnet-5-5, previous baseline | Medium | 7.30 s | 14.45 s |
| Codex / gpt-5.6-luna | Low | 15.77 s | 19.96 s |
| Codex / gpt-5.6-luna | Medium | 16.70 s | 20.23 s |
| Claude Code / Haiku | Low | 11.42 s | 18.29 s |

None of the measured alternatives beat the earlier Sonnet result. These are individual observations, not a model ranking. Sonnet was not rerun in this round. Codex's JSON transport first exposes the note through an agent-message event, so these numbers measure the installed app route, not raw server token latency.

Reproduce with the built executable: `--notebook-benchmark codex-low --annotation-proof-output <isolated-output-folder>`, replacing the variant with `codex-medium` or `haiku`. The entry point always constructs synthetic inputs from the repository fixture. The production notebook path still requests Medium effort.

- codex-low: [timing](../artifacts/notebooks-round1/live-codex-low/timing.json), [stream proof](../artifacts/notebooks-round1/live-codex-low/stream-proof.json), [note](../artifacts/notebooks-round1/live-codex-low/notebook-note.json).
- codex-medium: [timing](../artifacts/notebooks-round1/live-codex-medium/timing.json), [stream proof](../artifacts/notebooks-round1/live-codex-medium/stream-proof.json), [note](../artifacts/notebooks-round1/live-codex-medium/notebook-note.json).
- haiku: [timing](../artifacts/notebooks-round1/live-haiku/timing.json), [stream proof](../artifacts/notebooks-round1/live-haiku/stream-proof.json), [note](../artifacts/notebooks-round1/live-haiku/notebook-note.json).

## Gates

| Command | Result |
|---|---|
| `cargo fmt --all --check` | PASS, exit 0, 5.49 s |
| `cargo clippy --workspace --locked --all-targets -- -D warnings` | PASS, exit 0, 7.72 s |
| `cargo clippy --workspace --locked --all-targets --features asr-runtime -- -D warnings` | PASS, exit 0, 15.53 s |
| `cargo test --workspace --locked` | PASS, exit 0, 34.28 s |
| `cargo test --workspace --locked --features asr-runtime` | PASS, exit 0, 58.29 s |
| `cargo build --locked` | PASS, exit 0, 18.19 s |
| `python tools/mcp-smoke.py target/x86_64-pc-windows-gnu/debug/sidenote.exe` | PASS, exit 0, 1.18 s |

Both test modes pass 237 top-level tests: 1 domain unit, 14 domain integration, 178 app library and 44 app binary tests. No tests were ignored. Gate logs and exit codes are in [gate results](../artifacts/notebooks-round1/gates/results.json).

## Render and interaction proof

The same set is regenerated: 16 notebook app renders at 1280x800 and 640x800, light and dark; 28 annotation regression renders; and 4 HTML renders in Edge headless at 1280 and 390 wide, light and dark. Every app tree has zero layout warnings. The four UI exports are byte-identical and contain two unique approved PNGs, each appearing once, checked byte-for-byte and by hashes before isolated state cleanup. HTML has no scripts, external URLs, absolute paths or Sidenote IDs. Edge records zero HTTP(S) page requests.

Offscreen interactions verify Alt+Down, Alt+Up, drag, saved title editing, and entry into mandatory review when a capture note is missing. No model jobs are started by these interaction tests. No visible browser or desktop window is opened.

Evidence: automated tests and offscreen renders, rung 4; real synthetic CLI runs, rung 5.

## Commits

- `4729816` fixes shared screens and notebook-wide numbering, with synthetic tests and benchmarks.
- The UI/report commit adds compact rows, thumbnails and updated render assertions. Its hash is in the final handoff.

Both commits use `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.

## PNG paths

- `E:/Personal-Projects/hex-meetings/.worktrees/annotations-notebook/artifacts/notebooks-round1/notebooks-1280-light.png`
- `E:/Personal-Projects/hex-meetings/.worktrees/annotations-notebook/artifacts/notebooks-round1/notebook-1280-light.png`
- `E:/Personal-Projects/hex-meetings/.worktrees/annotations-notebook/artifacts/notebooks-round1/notebook-note-1280-light.png`
- `E:/Personal-Projects/hex-meetings/.worktrees/annotations-notebook/artifacts/notebooks-round1/share-html-1280-light.png`
- `E:/Personal-Projects/hex-meetings/.worktrees/annotations-notebook/artifacts/notebooks-round1/share-html-390-light.png`
- `E:/Personal-Projects/hex-meetings/.worktrees/annotations-notebook/artifacts/notebooks-round1/share-html-1280-dark.png`

Round 1 started 2026-11-11T06:59:46.7724365+05:30. Final elapsed time is recorded in the handoff.
