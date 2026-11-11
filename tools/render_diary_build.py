"""Verify Diary through native offscreen renders and isolated synthetic fixtures only."""
import argparse
from concurrent.futures import ThreadPoolExecutor
from datetime import datetime, timedelta
import hashlib
import json
import os
import shutil
from pathlib import Path
import subprocess
import struct
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "artifacts/diary-build"
TODAY = "2026-11-11"
ENV = os.environ.copy()
ENV["PATH"] = r"E:\Personal-Projects\tools\hex-meetings-gnu\w64devkit\bin;" + ENV["PATH"]
ENV["CARGO_TARGET_DIR"] = str(ROOT / "target")
FLAGS = subprocess.CREATE_NO_WINDOW


def save(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2, ensure_ascii=False), encoding="utf-8")


def fingerprint(values):
    value = 0xcbf29ce484222325
    for item in values:
        raw = str(item).encode("utf-8")
        for byte in len(raw).to_bytes(8, "little") + raw:
            value = ((value ^ byte) * 0x100000001b3) & 0xffffffffffffffff
    return f"{value:016x}"


def fixture(state, case):
    for folder in ["appdata", "localappdata", "peers", "meetings"]:
        (state / folder).mkdir(parents=True, exist_ok=True)
    save(state / "config.json", dict(version=1, features={k: True for k in ["tasks", "memory", "daily_check", "projects", "other_devices"]}))
    save(state / "projects.json", dict(projects=["Paper workshop"], aliases={}))
    if case == "empty":
        return state / "meetings"
    meetings = [
        (TODAY, "09:30", "Field guide review", 32, ["Mira Vale", "Theo Moss"], "The field guide is ready for a small workshop. The team chose a folded format and kept the examples short."),
        (TODAY, "11:00", "", 18, ["Ivo Reed"], ""),
        (TODAY, "14:00", "Workshop planning", 45, ["Mira Vale", "Nell Finch", "Theo Moss"], "The workshop will use paper samples. The venue reply and sample pack still need a follow-up."),
        ("2026-11-10", "10:00", "Sample pack check", 26, ["Theo Moss", "Ivo Reed"], "The sample pack needs one final copy check. A revised print proof is due this week."),
        ("2026-11-10", "15:00", "Invitation review", 22, ["Mira Vale", "Nell Finch"], "The invitation is shorter and the reply date is clear."),
        ("2026-11-09", "11:00", "Paper prototype review", 38, ["Ivo Reed", "Nell Finch"], "The folded prototype is easier to follow. The next version will add a quick start page."),
        ("2026-11-06", "09:00", "Workshop outline", 30, ["Theo Moss", "Mira Vale"], "The team outlined a hands-on workshop with two short exercises."),
    ]
    if case == "history": meetings = [((datetime.fromisoformat(TODAY)-timedelta(days=n)).date().isoformat(), "09:30", f"Paper archive review {n}", 20, ["Mira Vale"], "Review the archived paper sample.") for n in range(101)]
    if case == "untitled": meetings = [meetings[1]]
    if case == "future": meetings = [("2026-12-03", "10:00", "Future paper workshop", 25, ["Mira Vale"], "")]
    if case == "long": meetings[0] = (*meetings[0][:2], "Field guide review: " + "workshop readiness and the paper prototype for the next set of sample reviews " * 5, *meetings[0][3:])
    catalog = []
    for n, (day, clock, title, minutes, people, context) in enumerate(meetings):
        at = datetime.fromisoformat(day + "T" + clock + ":00+05:30")
        mid = str(int(at.timestamp() * 1000)) + "-1"
        specs = {
            0: [("Read the workshop outline", None, "todo", None), ("Review the invitation copy", TODAY, "review", None), ("Check the print quantity", "2026-11-10", "todo", None), ("Check the quick start page", "2026-11-13", "todo", None)],
            2: [("Read the meeting notes", None, "todo", None), ("Confirm the room booking", "2026-11-12", "waiting", "The venue team's reply"), ("Prepare the sample pack", "2026-11-13", "todo", None)],
            3: [("Share the revised print proof", "2026-11-12", "waiting", "The print proof review"), ("Confirm who sends the sample pack", None, "todo", None)],
        }.get(n, []) if case not in ["untitled", "future"] else []
        if case == "stress" and n == 0: specs += [(f"Review paper sample {i + 1}", "2026-11-10", "todo", None) for i in range(60)]
        todos = [dict(id=f"task-{i}", task=text, title=text, description="Review the synthetic paper workshop material.", owner="Mira Vale", due=due, status=status, waiting_on=dict(text=waiting) if waiting else None) for i, (text, due, status, waiting) in enumerate(specs)]
        if case == "untitled":
            save(state / "meetings" / mid / "manifest.json", dict(title="11:00", status="complete", created_at_ms=int(at.timestamp()*1000), duration_ms=minutes*60000))
        else:
            save(state / "meetings" / mid / "manifest.json", dict(status="complete", created_at_ms=int(at.timestamp()*1000), duration_ms=minutes*60000))
        summary = dict(schema=2, title=title, people=people, context=[context] if context else [], todo=todos,
                       topics=[dict(title="Outcome", points=[dict(type="decided", text="Use the folded paper format.", at=["00:05"]), dict(type="decision", text="Use the folded paper format.", at=["00:05"])])] if n == 0 and case not in ["untitled", "future"] else [])
        save(state / "meetings" / mid / "english-summary.json", summary)
        save(state / "meetings" / mid / "tasks.json", dict(meeting_project="Paper workshop", tasks=[dict(i=i, project="Paper workshop", mine=True, new=False) for i in range(len(todos))]))
        catalog.append((mid, summary))
    if case in ["untitled", "future"]: return state / "meetings"
    if case == "features-off":
        save(state / "config.json", dict(version=1, features={k: k not in ["tasks", "memory"] for k in ["tasks", "memory", "daily_check", "projects", "other_devices"]}))
    stamp = TODAY + "T08:00:00+05:30"
    first = catalog[0][0]
    sources = [dict(type="meeting", reference=f"[[{first}@00:05]]")]
    records = [dict(id="synthetic-memory-fold", type="decision", text="Use the folded paper format.", project="Paper workshop", people=[], sources=sources, first_seen="2026-11-09", last_confirmed=None, status="active", basis="source_supported", supersedes=None, updated_at=stamp, source_fingerprint="synthetic", source_fingerprints=["synthetic"])]
    proposals = [dict(id=f"synthetic-proposal-{i}", op="add", target_id=None, text=text, record_type="decision", project="Paper workshop", people=[], sources=sources, rationale="Review the meeting outcome.", target_updated_at=None, status="pending", created_at=stamp) for i, text in enumerate(["Keep workshop exercises short.", "Add a quick start page.", "Share the sample pack before Friday."])]
    save(state / "memory/records.json", dict(schema_version=1, records=records, proposals=proposals, events=[dict(record_id=records[0]["id"], operation="accept_proposal", at=stamp, proposal_id=None)], migration=dict(version=1, source_path="synthetic", fingerprint="synthetic", accepted_at=stamp)))
    findings = []
    for n, index in [(0, 1), (0, 2), (2, 1), (3, 1)]:
        mid, summary = catalog[n]
        subject = f"task:{mid}:{index}"
        findings.append(dict(id=f"synthetic-finding-{n}-{index}", kind="missing_followup", severity="high" if index == 1 else "medium", title="DEC-123 revision abcdef123456 should never reach Diary", explanation="Raw internal evidence stays in the full report.", evidence=[f"[[{mid}@00:05]]"], subject_refs=[subject], subject_keys=[subject + "@" + fingerprint([summary["todo"][index]["task"]])], material_state="synthetic", evidence_keys=[f"[[{mid}@00:05]]"], change="changed" if n == 2 else "new"))
    findings.append(dict(id="synthetic-finding-memory", kind="conflicting_decisions", severity="high", title="Internal memory identity", explanation="Review source details.", evidence=[f"[[{first}@00:05]]"], subject_refs=["memory:synthetic-memory-fold"], subject_keys=["memory:synthetic-memory-fold"], material_state="synthetic", evidence_keys=[f"[[{first}@00:05]]"], change="changed"))
    findings.append(dict(id="synthetic-finding-unmapped", kind="other", severity="medium", title="Internal project reference", explanation="Review the original report.", evidence=["workshop.md"], subject_refs=["project:Paper workshop|open-question"], subject_keys=["project:Paper workshop|open-question"], material_state="synthetic", evidence_keys=["workshop.md"], change="new"))
    def report(day, items):
        return dict(schema_version=2, date=day, runner="Synthetic fixture", prompt_bytes=0, elapsed_seconds=0, meeting_count=len(catalog), task_count=9, kb=dict(revision="0"*40, captured_at=stamp, path=str(state / "synthetic-kb"), stale=False), warnings=[], memory_updated=False, findings=items, brief=["Internal raw brief must not appear in Diary."], comparison=dict(baseline_date=None, complete=True, new=[], changed=[], carried=[], resolved=[], unmatched=[]), day_summary="The sample pack and invitation are nearly ready. The remaining work is a final copy check and print proof.", day_summary_date="2026-11-10")
    save(state / f"checks/{TODAY}.json", report(TODAY, findings))
    save(state / "checks/2026-11-09.json", report("2026-11-09", findings[:3]))
    if case == "broken": (state / "checks/2026-11-09.json").write_text("{invalid")
    return state / "meetings"


def nodes(value):
    if isinstance(value, dict):
        yield value
        for child in value.values(): yield from nodes(child)
    elif isinstance(value, list):
        for child in value: yield from nodes(child)


def render(exe, name, width=1440, theme="dark", case="default", events=None, scene="diary", height=1500, scale=1, expected=None):
    state = Path(tempfile.mkdtemp(prefix=name + "-", dir=ROOT / "target"))
    meetings = fixture(state, case)
    if scene.startswith("settings-") or scene == "people-team-editor":
        shutil.copy2(ROOT / "tools/ui-fixtures/hex.json", state / "hex-fixture.json")
    if scene == "people-team-editor":
        save(state / "teams.json", dict(teams=[dict(name="Paper workshop team", members=["Mira Vale", "Theo Moss"], aliases={})]))
    base = OUT / f"{name}-{width}-{theme}"
    env = ENV | {"APPDATA": str(state/"appdata"), "LOCALAPPDATA": str(state/"localappdata"), "SIDENOTE_STATE_DIR": str(state)}
    script = OUT / "scripts" / (base.name + ".json")
    events = list(events or [])
    if not any(e.get("type") == "mark" and e.get("name") == "end" for e in events):
        events.append(dict(type="mark", name="end", t=max([e["t"] for e in events] + [0])+2.0))
    save(script, dict(events=events))
    args = [str(exe), "--meetings-dir", str(meetings), "--data-dir", str(state), "--peers-dir", str(state/"peers"), "--scene", scene, "--theme", theme, "--size", f"{width}x{height}", "--scale", str(scale), "--snap", str(base)+".png", "--tree", str(base)+".json", "--script", str(script), "--at", "mark:end"]
    started = time.monotonic()
    with Path(str(base)+".log").open("w") as log:
        subprocess.run(args, cwd=ROOT, env=env, stdout=log, stderr=log, creationflags=FLAGS, timeout=180, check=True)
    tree = json.loads(Path(str(base)+".json").read_text(encoding="utf-8-sig"))
    assert not tree["warnings"], (name, tree["warnings"])
    all_nodes = list(nodes(tree))
    keys = {n.get("key") for n in all_nodes}
    text = " ".join(str(n.get("text", "")) for n in all_nodes)
    if expected:
        for key in expected: assert key in keys, (name, key, sorted(str(k) for k in keys))
    elif scene == "diary":
        assert "check-top" in keys
        assert "meeting-breadcrumb" not in keys and "memory-breadcrumb" not in keys
        assert "DEC-123" not in text and "abcdef123456" not in text and "Internal raw brief" not in text
        assert chr(0x2014) not in text
        if width >= 1100: assert "diary-rail-scroll" in keys
        else: assert "diary-week-toggle" in keys
        if name == "diary" and width == 390: assert "diary-calendar" not in keys
        if name == "diary-read-error-folded":
            assert "diary-calendar" not in keys and "Could not load follow-ups" in text
        if case == "empty": assert "No meetings yet" in text
        if case == "untitled": assert "Untitled meeting" in text
        if case == "features-off": assert not ({"diary-due-all", "diary-waiting-all", "diary-memory-all"} & keys)
        for n in all_nodes:
            if str(n.get("key", "")).startswith("diary-") and "rect" in n:
                x, y, w, h = n["rect"]
                assert x >= -1 and x+w <= width+1, (name, n.get("key"), n["rect"])
    if name == "diary-stress-all":
        assert len({n["key"] for n in all_nodes if str(n.get("key", "")).removeprefix("diary-review-").isdigit()}) > 50, "Full follow-up list lost overflow items"
    if name == "diary-history":
        assert any(n.get("key") == "diary-2026-08-03" and n.get("rect") and 52 <= n["rect"][1] < height for n in all_nodes), "Old selected day was not revealed"
    if name == "diary-rail-short":
        assert any(n.get("key") == "diary-proposal-0" and n.get("rect") and 52 <= n["rect"][1] < height for n in all_nodes), "Lower rail sections were not reachable"
        assert any(n.get("key") == "diary-2026-11-11" and n.get("rect") and 52 <= n["rect"][1] < 200 for n in all_nodes), "Rail scrolling moved the timeline"
    print("PASS " + base.name, flush=True)
    return dict(png=str(base)+".png", tree=str(base)+".json", width=width, theme=theme, case=case, scene=scene, seconds=round(time.monotonic()-started, 2), sha256=hashlib.sha256(Path(str(base)+".png").read_bytes()).hexdigest())


def tap(key, t=0.5): return dict(type="tap", key=key, t=t)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("exe", type=Path)
    parser.add_argument("--quick", action="store_true")
    parser.add_argument("--audit", action="store_true")
    parser.add_argument("--settings", action="store_true")
    parser.add_argument("--case", default=None)
    parser.add_argument("--interactions", action="store_true")
    args = parser.parse_args()
    OUT.mkdir(parents=True, exist_ok=True)
    exe = args.exe.resolve()
    if args.audit:
        manifest = json.loads((OUT / "render-manifest.json").read_text())
        binary_hash = hashlib.sha256(exe.read_bytes()).hexdigest()
        assert manifest["binary_sha256"] == binary_hash, "Renders must use the final worktree binary"
        required = {f"{name}-{width}-{theme}.png" for name in ["diary", "diary-outcomes", "diary-calendar", "diary-rail", "diary-empty"] for width in [1440, 1280, 390] for theme in ["light", "dark"]}
        required |= {f"diary-week-open-390-{theme}.png" for theme in ["light", "dark"]}
        found = set()
        for shot in manifest["rows"]:
            raw = Path(shot["png"]).read_bytes()
            assert raw[:8] == b"\x89PNG\r\n\x1a\n"
            assert struct.unpack(">II", raw[16:24]) == (shot["width"], 1500)
            assert hashlib.sha256(raw).hexdigest() == shot["sha256"]
            tree = json.loads(Path(shot["tree"]).read_text(encoding="utf-8-sig"))
            assert not tree["warnings"]
            found.add(Path(shot["png"]).name)
        assert required <= found, sorted(required - found)
        interaction = json.loads((OUT / "interaction-manifest.json").read_text())
        for shot in interaction:
            assert hashlib.sha256(Path(shot["png"]).read_bytes()).hexdigest() == shot["sha256"]
            tree = json.loads(Path(shot["tree"]).read_text(encoding="utf-8-sig"))
            assert not tree["warnings"]
            if Path(shot["png"]).name.startswith("diary-history-"):
                assert any(n.get("key") == "diary-2026-08-03" and n.get("rect") and 52 <= n["rect"][1] < 1500 for n in nodes(tree))
        gates = json.loads((OUT / "gates.json").read_text())
        assert set(gates) == {"fmt", "clippy", "clippy-asr", "test", "test-asr", "build", "smoke", "settings"}
        assert all(row["exit_code"] == 0 for row in gates.values())
        save(OUT / "verification.json", dict(binary_sha256=binary_hash, required_states=len(required), diary_renders=len(manifest["rows"]), interaction_renders=len(interaction), gate_count=len(gates), warnings=0, verified_at=datetime.utcnow().isoformat()+"Z"))
        print(f"PASS: {len(manifest['rows'])} Diary PNGs, all {len(required)} required state/theme/width combinations, {len(interaction)} extra checks and all eight gates", flush=True)
        return
    if args.interactions:
        cases = [
            ("diary-route-task", [tap("diary-needs-0")], ["tasks-pane", "tasks-panel", "diary-return"]),
            ("diary-route-due", [tap("diary-due-all")], ["tasks-pane", "diary-task-filter-clear", "diary-return"]),
            ("diary-route-waiting", [tap("diary-waiting-0")], ["tasks-pane", "tasks-panel", "diary-return"]),
            ("diary-route-waiting-all", [tap("diary-waiting-all")], ["tasks-pane", "diary-task-filter-clear"]),
            ("diary-route-memory", [tap("diary-needs-2")], ["memory-pane", "diary-return"]),
            ("diary-route-proposal", [tap("diary-proposal-0")], ["memory-pane", "diary-return"]),
            ("diary-route-memory-review", [tap("diary-memory-all")], ["memory-pane", "diary-return"]),
            ("diary-route-all", [tap("diary-other-followups")], ["diary-review-close"]),
            ("diary-route-return", [tap("diary-due-all", .4), tap("diary-return", 1.0)], ["diary-timeline", "diary-calendar"]),
            ("diary-calendar-keyboard", [tap("diary-month-next", .3), dict(type="key", name="Tab", t=.5), dict(type="key", name="Right", t=.7), dict(type="key", name="Enter", t=1.0)], ["diary-2026-11-12"]),
            ("diary-calendar-escape", [tap("diary-month-next", .3), dict(type="key", name="Tab", t=.5), dict(type="key", name="Escape", t=.7)], ["diary-calendar-grid-true"]),
        ]
        rows = [render(exe, name, 1440, "light", events=events, expected=expected) for name, events, expected in cases]
        rows.append(render(exe, "diary-stress-all", 1440, "light", case="stress", events=[tap("diary-other-followups")], expected=["diary-review-close"]))
        rows.append(render(exe, "diary-history", 1440, "light", case="history", events=[tap("diary-month-prev", .3), tap("diary-month-prev", .6), tap("diary-month-prev", .9), tap("diary-date-2026-08-03", 1.2)], expected=["diary-2026-08-03"]))
        rows.append(render(exe, "diary-feature-flags", 1440, "light", case="features-off"))
        rows.append(render(exe, "diary-read-error-folded", 390, "light", case="broken"))
        rows.append(render(exe, "diary-text-200", 1440, "light", scale=2))
        for width in [1099, 1100, 1920]:
            for theme in ["light", "dark"]: rows.append(render(exe, "diary-breakpoint", width, theme))
        for theme in ["light", "dark"]:
            rows.append(render(exe, "diary-rail-short", 1440, theme, height=800, events=[dict(type="wheel", x=1250, y=600, dy=-1200, t=.4), dict(type="mark", name="end", t=1.4)]))
        save(OUT/"interaction-manifest.json", rows)
        return
    if args.settings:
        scenes = ["general", "recording", "summaries", "tasks", "prompts", "people", "hex", "helpers", "speech", "helper-advanced", "appearance", "sync", "updates", "search", "recording-expanded", "summaries-expanded", "tasks-expanded", "hex-expanded", "helper-advanced-expanded", "sync-expanded", "updates-expanded", "helpers-expanded"]
        def run_settings(scene):
            events = json.loads((ROOT / "tools/ui-scripts/settings-search.json").read_text())["events"] if scene == "search" else None
            return render(exe, "settings-"+scene, 1280, "light", "empty", events, "settings-"+scene, 800)
        with ThreadPoolExecutor(max_workers=2) as pool: rows = list(pool.map(run_settings, scenes))
        rows.append(render(exe, "settings-team-editor", 1280, "light", "empty", scene="people-team-editor", height=800, events=[tap("team-advanced")]))
        subprocess.run(["python", "tools/settings-reachability.py", str(OUT)], cwd=ROOT, env=ENV, check=True)
        save(OUT/"legacy-settings-manifest.json", rows)
        return
    cases = [("diary", "default", None), ("diary-outcomes", "default", [tap("diary-fold-2026-11-11:outcomes")]), ("diary-calendar", "default", [tap("diary-date-2026-11-08")]), ("diary-rail", "default", None), ("diary-week-open", "default", [tap("diary-week-toggle")]), ("diary-empty", "empty", None), ("diary-untitled", "untitled", None), ("diary-long", "long", None), ("diary-future", "future", None), ("diary-stress", "stress", None), ("diary-read-error", "broken", None)]
    if args.case: cases = [c for c in cases if c[0] == args.case]
    if args.quick and not args.case: cases = cases[:1]
    jobs = []
    for name, case, events in cases:
        for width in ([1440] if args.quick else [1440, 1280, 390]):
            if name == "diary-week-open" and width >= 1100: continue
            selected_events = events
            if width == 390 and name == "diary-calendar": selected_events = [tap("diary-week-toggle", 0.3), tap("diary-date-2026-11-08", 0.8)]
            if width == 390 and name in ["diary-rail", "diary-read-error"]: selected_events = [tap("diary-week-toggle")]
            for theme in (["dark"] if args.quick else ["dark", "light"]): jobs.append((name, width, theme, case, selected_events))
    with ThreadPoolExecutor(max_workers=2) as pool: rows = list(pool.map(lambda job: render(exe, *job), jobs))
    save(OUT/"render-manifest.json", dict(binary_sha256=hashlib.sha256(exe.read_bytes()).hexdigest(), rows=rows))
    print(f"PASS: {len(rows)} native Diary renders", flush=True)

if __name__ == "__main__": main()
