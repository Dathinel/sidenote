"""Shows our English transcript and summary in the Hex app by replacing Hex's own copies.

Hex's Meetings pane reads the transcript from transcript.ndjson, the summary and title from
manifest.json "notes" (the summary carries every section; "chapters" is cleared), and "Reveal Markdown" opens
manifest.json "export_path". This rewrites all four from english-transcript.md and english-summary.json (rendered by hex-meeting-render.py).

Reverse path: the first run copies Hex's originals into <meeting>/hex-original/; copy them back.
Hex overwrites the summary again if you press "Regenerate summary" or turn on
"Summarize with OpenCode"; rerun this script afterwards.

Usage: python hex-meeting-publish.py <meeting folder>
"""
import hashlib
import importlib.util
import json
import os
import re
import shutil
import sys
import time
from pathlib import Path

NOTES_START, NOTES_END = "<!-- hex:notes -->", "<!-- /hex:notes -->"
# The translation model sometimes drops the milliseconds.
LINE = re.compile(r"^\[(\d+):(\d+):(\d+)(?:\.(\d+))?\] (\S+) \(([^)]*)\): ?(.*)$")


def read(path):
    return Path(path).read_text(encoding="utf-8")


def write_atomic(path, text):
    guard = renderer()
    path = guard.no_links(path)
    temporary = guard.no_links(Path(str(path) + ".tmp"))
    temporary.write_text(text, encoding="utf-8", newline="\n")
    os.replace(temporary, path)


def clock(ms):
    seconds = ms // 1000
    return f"{seconds // 60:02d}:{seconds % 60:02d}"


def entries_from(meeting):
    """english-transcript.md lines become Hex transcript entries; You is the microphone."""
    segments = []
    evidence = meeting / "audio-evidence.json"
    if evidence.exists():
        segments = [(round(s["Start"] * 1000), round(s["End"] * 1000), s.get("Speaker"))
                    for s in json.loads(read(evidence)).get("segments", [])]
    used = set()
    rows = []
    for raw in read(meeting / "english-transcript.md").splitlines():
        match = LINE.match(raw.strip())
        if match:
            h, m, s, frac, speaker, _language, text = match.groups()
            start = ((int(h) * 60 + int(m)) * 60 + int(s)) * 1000 + int((frac or "0").ljust(3, "0")[:3])
            # The audio segment this line came from: same speaker, nearest start within a second.
            near = [(abs(a - start), i) for i, (a, _, who) in enumerate(segments)
                    if i not in used and who == speaker and abs(a - start) < 1000]
            end = None
            if near:
                index = min(near)[1]
                used.add(index)
                start, end = segments[index][0], segments[index][1]
            rows.append([start, speaker, text.strip(), end])
        elif raw.strip() and rows and not raw.startswith("#"):
            rows[-1][2] += " " + raw.strip()
    if not rows:
        raise SystemExit("No transcript lines in english-transcript.md")
    rows.sort(key=lambda row: row[0])
    others = {row[1] for row in rows if not row[1].startswith("You")}
    entries = []
    for index, (start, speaker, text, end) in enumerate(rows):
        following = rows[index + 1][0] if index + 1 < len(rows) else None
        end = end or (following if following and following > start else None) or start + 400 * max(1, len(text.split()))
        mine = speaker.startswith("You")
        # Hex shows only You and Them, so name the other speaker when there is more than one.
        shown = text if mine or len(others) < 2 else f"[{speaker}] {text}"
        entries.append({"source": "microphone" if mine else "system",
                        "start_ms": start, "end_ms": max(end, start), "text": shown})
    return entries


def renderer():
    spec = importlib.util.spec_from_file_location("hex_meeting_render", Path(__file__).with_name("hex-meeting-render.py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def main():
    meeting = renderer().no_links(Path(sys.argv[1]))
    manifest_path = meeting / "manifest.json"
    if not manifest_path.exists():
        print(f"Not a Hex meeting folder (no manifest.json): {meeting}")
        return
    manifest = json.loads(read(manifest_path))
    if manifest.get("status") != "complete":
        raise SystemExit(f"Hex has not finished this meeting (status {manifest.get('status')}); not touching it.")
    if not (meeting / "english-summary.json").exists():
        raise SystemExit("No english-summary.json; rerun hex-meeting-english.ps1 -NotesOnly for this meeting.")

    render = renderer()
    export = render.safe_export(manifest["export_path"]) if manifest.get("export_path") else None
    page = render.render(meeting)  # english-summary.md and the viewer page
    data, meta, _ = render.load(meeting)
    entries = entries_from(meeting)

    original = meeting / "hex-original"
    if not original.exists():
        original.mkdir()
        for name in ("manifest.json", "transcript.ndjson", "transcript.md"):
            if (meeting / name).exists():
                shutil.copy2(meeting / name, original / name)
        if export and export.exists():
            shutil.copy2(export, original / export.name)

    # Transcript: the pane and `hex meetings show` read transcript.ndjson; transcript.md must exist beside it.
    # Hex shows a calendar, call or user-given title over the notes title; match it in the files.
    display = meta["title"]
    write_atomic(meeting / "transcript.ndjson",
                 "".join(json.dumps(entry, ensure_ascii=False) + "\n" for entry in entries))
    labels = {"microphone": "You", "system": "Them"}
    write_atomic(meeting / "transcript.md", f"# {display}\n\n" + "".join(
        f"**{clock(e['start_ms'])} {labels[e['source']]}**  {e['text']}\n" for e in entries))

    # Summary: Hex draws notes.summary as plain text, so it gets the plain-text layout; no chapter list.
    transcript_chars = sum(len(e["text"]) for e in entries)
    manifest["notes"] = {
        "title": data["title"],
        "summary": render.hex_text(data),
        "generated_at_ms": int(time.time() * 1000),
        "elapsed_ms": 0,
        "excerpt_chars": transcript_chars,
        "transcript_chars": transcript_chars,
    }
    manifest["notes_error"] = None
    manifest["chapters"] = []

    # Reveal Markdown: Hex's header, then our summary in its notes block, then the English transcript folded away.
    if export and export.exists():
        old = read(export)
        header = old.split("\n## Transcript", 1)[0]
        header = re.sub(re.escape(NOTES_START) + r".*?" + re.escape(NOTES_END) + r"\n*", "", header, flags=re.S)
        header = re.sub(r"\A# .*", lambda _: f"# {display}", header).rstrip()
        turns = "".join(f"**{labels[e['source']]}** `{clock(e['start_ms'])}`  \n{e['text']}\n\n" for e in entries)
        markdown = (f"{header}\n\n{NOTES_START}\n{render.markdown(data, meta, with_title=False)}\n"
                    f"<sub>English notes from the Hex meeting pipeline. [Open the meeting page]({page.as_uri()})</sub>\n"
                    f"{NOTES_END}\n\n## Transcript\n\n<details>\n<summary>Show the full transcript ({len(entries)} lines)</summary>\n\n"
                    f"{turns}</details>\n")
        write_atomic(render.safe_export(export), markdown)
        manifest["export_sha256"] = hashlib.sha256(export.read_bytes()).hexdigest()

    write_atomic(manifest_path, json.dumps(manifest, ensure_ascii=False, indent=2) + "\n")
    print(f"Hex now shows the English transcript ({len(entries)} lines) and summary for {meeting.name}."
          f" Originals: {original}")
    if export:
        print(f"Markdown: {export}")
    print(f"Meeting page: {page}")


if __name__ == "__main__":
    main()
