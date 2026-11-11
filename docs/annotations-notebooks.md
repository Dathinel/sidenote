Decisions needed: none

# Annotation notebooks and sharing

Implemented in `feat/annotations-notebook`, based on `release/dev12` at `d1ec063`. Builds use this worktree’s own target directory. No push or version bump; version remains `0.0.1-dev.12`.

## What shipped

- Annotations now has a Notebooks section above captures, with New notebook. Capture pages offer Add to notebook.
- Notebook title and optional one-line purpose are editable. Capture cards support adding, removing, dragging and Alt+Up/Alt+Down. A reorder also reorders an existing current note without another model call, preserving edits.
- Adding or removing captures retains the previous note but marks it out of date and disables sharing until rewritten.
- Write notebook note uses the selected Annotation notes agent and model from Models and agents. It sends existing per-capture note prose only, with no images. A capture without a valid note first enters the existing image review and explicit Send for note step. The notebook continues after that note is saved.
- The notebook note has an overall title, a 2 to 4 sentence summary, one section per capture in chosen order, approved frames and their matching numbered callouts. Questions and to-dos are merged and deduplicated; conflicting questions stay distinct. Legacy inline uncertainty is retained at the end.
- All visible note text can be edited and saved locally. Notebook edits do not modify scrawl bundles or per-capture notes.
- Share works for capture and notebook notes. It uses a native save dialog, a title-derived default filename, then Copy file and Show in folder. Copy file uses a native file clipboard payload.
- HTML has inline CSS, embedded approved marked PNG bytes, responsive side-by-side/below-image callouts, light/dark media styles, print styles and the muted Made with Sidenote footer. It contains no scripts, external URLs, local paths or dumped internal IDs. Saving inside a scrawl bundle or over another file format is rejected.

## Live proof and speed

Final live run: **7.30 seconds to first generated text, 14.45 seconds to a complete valid notebook note**. Requested model: `claude-sonnet-5-5`, Medium effort. Three synthetic captures; zero images sent in the notebook pass.

The under-five-second aim was not reached. Four live calls were used during input, streaming and wording corrections. All inputs were synthetic copies of the fixture bundle, using the existing synthetic capture note; they share image content. These are individual observations, not a representative benchmark for three distinct real recordings.

| Run | First text | Complete |
|---|---:|---:|
| live | 16.41 s | 26.41 s |
| live-compact | 11.11 s | 21.91 s |
| live-streaming | 7.36 s | 16.32 s |
| live-final | 7.30 s | 14.45 s |

The final prompt removes metadata from the synthesis payload, shares identical prose once, and explicitly rejects unsupported benefits or completeness claims. This last correction removed a generated claim that the listed materials were all that was needed and made the task quick. The final UI fixture is the unedited final live result. Partial JSON previews include headings and paragraphs as well as title and summary, while excluding reference keys.

[Final timing](../artifacts/notebooks/live-final/timing.json), [stream events](../artifacts/notebooks/live-final/stream-proof.json), [raw note](../artifacts/notebooks/live-final/notebook-note.json), [actual exported HTML](../artifacts/notebooks/notebook.html).

## Gate results

| Command | Result |
|---|---|
| `cargo fmt --all --check` | PASS, exit 0, 1.74 s |
| `cargo clippy --workspace --locked --all-targets -- -D warnings` | PASS, exit 0, 3.16 s |
| `cargo clippy --workspace --locked --all-targets --features asr-runtime -- -D warnings` | PASS, exit 0, 3.05 s |
| `cargo test --workspace --locked` | PASS, exit 0, 20.45 s |
| `cargo test --workspace --locked --features asr-runtime` | PASS, exit 0, 20.62 s |
| `cargo build --locked` | PASS, exit 0, 0.77 s |
| `python tools/mcp-smoke.py E:/Personal-Projects/hex-meetings/.worktrees/annotations-notebook/target/x86_64-pc-windows-gnu/debug/sidenote.exe` | PASS, exit 0, 0.91 s |

Both test modes pass 234 top-level tests: 1 domain unit, 14 domain integration, 175 app library and 44 app binary tests. Subprocess self-checks report additional passing summaries. There are no ignored or failing tests.

Proofs passed: 16 notebook app renders at 1280x800 and 640x800 in both themes; 28 existing annotation regression renders; 4 exported HTML renders in isolated Edge headless profiles at 1280 and 390 wide. All app trees have zero layout warnings. Edge media emulation explicitly selects light and dark, with independent PNG pixel checks. No Chrome or visible browser window was used.

The four UI-generated exports are byte-identical across app themes and sizes. Every one contains only the six approved marked PNGs, checked byte-for-byte and by hashes before temporary state cleanup. The Edge page has zero script elements and zero HTTP(S) page requests.

Actual offscreen interactions passed: Alt+Down, Alt+Up, drag, saved title editing, and mandatory image review when a capture note is missing. They started no model jobs. Native desktop save-dialog, clipboard and folder actions were compiled and wired but were not opened on the owner’s screen.

Evidence: code/tests/offscreen renders rung 4; actual synthetic Claude CLI proof rung 5.

## Commits

- ecf22e2 feat(ui): add notebook editing and HTML sharing
- 50d6416 fix(annotations): stream notebook sections and retain uncertainty
- dc14758 feat(annotations): add ordered notebooks and static HTML export
- The final proof commit contains the harnesses, live fixture, editor badge-label correction, card-height and prompt refinements, and this report. Its hash is in the final handoff.

Every commit ends with the requested Claude Opus 5.5 co-author trailer.

## Absolute PNG paths

- `E:/Personal-Projects/hex-meetings/.worktrees/annotations-notebook/artifacts/notebooks/notebooks-1280-light.png`
- `E:/Personal-Projects/hex-meetings/.worktrees/annotations-notebook/artifacts/notebooks/notebook-1280-light.png`
- `E:/Personal-Projects/hex-meetings/.worktrees/annotations-notebook/artifacts/notebooks/notebook-note-1280-light.png`
- `E:/Personal-Projects/hex-meetings/.worktrees/annotations-notebook/artifacts/notebooks/share-html-1280-light.png`
- `E:/Personal-Projects/hex-meetings/.worktrees/annotations-notebook/artifacts/notebooks/share-html-390-light.png`
- `E:/Personal-Projects/hex-meetings/.worktrees/annotations-notebook/artifacts/notebooks/share-html-1280-dark.png`

Elapsed at report generation: 88.9 minutes. Start 2026-11-11T05:28:27.967847+05:30, report 2026-11-11T06:57:22.959705+05:30. Cold build used about 10 minutes. The largest avoidable rework was repeated final checks while correcting labels and Edge color-scheme forcing.
