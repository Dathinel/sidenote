r"""Assigns each meeting to-do to a project and marks the ones that are mine.

Writes <meeting>\tasks.json: {"meeting_project", "tasks": [{"i", "project", "new", "mine"}]}.
Project names come from the configured organization tracking list (projects.yaml on origin/master) plus names
already used or added in the Meetings app (%APPDATA%\hex-meetings\projects.json). New names
remain suggestions in tasks.json until the user creates the project in Meetings.

Usage: hex-meeting-tasks.py <meeting folder>... | --all [--force]
"""
import json
import os
import re
import subprocess
import sys
from pathlib import Path
import importlib.util
_spec = importlib.util.spec_from_file_location("hex_pipeline_run", Path(__file__).with_name("hex-pipeline-run.py"))
pipeline = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(pipeline)

MEETINGS = Path(os.environ["APPDATA"]) / "hex" / "meetings"
APP = Path(os.environ["APPDATA"]) / "hex-meetings"
KNOWN = APP / "projects.json"
KB = os.environ.get("HEX_MEETING_KB_REPO", "")
MODEL = "gpt-6.1-sol"
ME = [os.environ.get("HEX_MEETING_OWNER_NAME", "You")] + os.environ.get("HEX_MEETING_OWNER_ALIASES", "You").split(";")

PROMPT = """You sort the to-dos from one meeting into projects for the recorder's own task list.

Known projects (use the exact name when one fits):
{projects}

Rules:
- Pick the project the work actually moves forward, judged from the meeting's context and discussion, not from keywords alone.
- If none fits, make a short new project name (2 to 5 words, sentence case, e.g. "Website V2 launch") and use the same name for related to-dos in this meeting. Prefer an existing name over a near-duplicate new one.
- "mine" is true when the owner is {owner} (shown as You in the transcript, or named {owner}), false when it is clearly someone else, null when the owner is not stated.
- meeting_project is the one project the meeting is mostly about.

Reply with only this JSON, no other text:
{{"meeting_project": "...", "tasks": [{{"i": 0, "project": "...", "new": false, "mine": null}}]}}

Meeting summary:
{summary}
"""


def kb_projects():
    if not KB:
        return []
    try:
        text = subprocess.run(["git", "-C", KB, "show", "origin/master:07-Planning/tracking/projects.yaml"],
                              capture_output=True, text=True, encoding="utf-8", timeout=30,
                              creationflags=subprocess.CREATE_NO_WINDOW).stdout
    except (OSError, subprocess.SubprocessError):
        return []
    # Top-level projects sit at four spaces; their milestones are indented further.
    return [m.group(1) for m in re.finditer(r'^    title: "([^"]+)"', text, re.M)]


def known():
    try:
        return json.loads(KNOWN.read_text(encoding="utf-8"))
    except (OSError, ValueError):
        return {"projects": [], "aliases": {}}


def classify(folder, force=False):
    folder = Path(folder)
    out = folder / "tasks.json"
    summary = json.loads((folder / "english-summary.json").read_text(encoding="utf-8"))
    todo = summary.get("todo") or []
    if out.exists() and not force:
        return "kept"
    if not todo:
        out.write_text(json.dumps({"meeting_project": None, "tasks": []}), encoding="utf-8")
        return "no to-dos"
    data = known()
    names = list(dict.fromkeys(kb_projects() + data["projects"]))
    brief = {k: summary.get(k) for k in ("title", "kind", "people", "context", "topics")}
    brief["todo"] = [{"i": i, **t} for i, t in enumerate(todo)]
    prompt = PROMPT.format(owner=", ".join(ME), projects="\n".join(f"- {n}" for n in names) or "- (none yet)",
                           summary=json.dumps(brief, ensure_ascii=False, indent=1))
    text = pipeline.codex(prompt, MODEL, 300)
    if "{" not in text:
        raise RuntimeError("Invalid task response")
    result = json.loads(text[text.index("{"):text.rindex("}") + 1])
    tasks = {t["i"]: t for t in result.get("tasks", []) if isinstance(t.get("i"), int) and 0 <= t["i"] < len(todo)}
    # Every to-do gets an entry; a missing one falls back to the meeting's project.
    fallback = result.get("meeting_project") or "Unsorted"
    result["tasks"] = [tasks.get(i, {"i": i, "project": fallback, "new": False, "mine": None}) for i in range(len(todo))]
    # Model names are suggestions. Only a user action registers a project.
    out.write_text(json.dumps(result, ensure_ascii=False, indent=2), encoding="utf-8")
    return f"{len(todo)} to-dos into {len({t['project'] for t in result['tasks']})} projects"


if __name__ == "__main__":
    args = [a for a in sys.argv[1:] if a != "--force"]
    force = "--force" in sys.argv
    folders = [p.parent for p in MEETINGS.glob("*/english-summary.json")] if args == ["--all"] else args
    failed = False
    for f in folders:
        try:
            print(f"{Path(f).name}: {classify(f, force)}")
        except Exception as error:
            failed = True
            if isinstance(error, pipeline.PipelineFailure):
                print(str(error), file=sys.stderr)
            print(f"{Path(f).name}: failed ({type(error).__name__}; detail bytes={len(str(error).encode())})")
    if failed:
        sys.exit(1)
