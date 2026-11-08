"""Generate wholly synthetic Northwind check/memory state. Never read AppData or call agents."""
import json
import sys
from datetime import date, timedelta
from pathlib import Path

MEETING = "1800000000000-1"
ROOT = Path(__file__).resolve().parents[1]


def fnv(parts):
    value = 0xCBF29CE484222325
    for part in parts:
        raw = part.encode("utf-8")
        for byte in len(raw).to_bytes(8, "little") + raw:
            value = ((value ^ byte) * 0x100000001B3) & 0xFFFFFFFFFFFFFFFF
    return f"{value:016x}"


def compact(value):
    return json.dumps(value, separators=(",", ":"), ensure_ascii=False)


def sources_fp(sources):
    return fnv(sorted({compact(source) for source in sources}))


def proposal_id(p):
    return fnv([compact(p["op"]), p.get("target_id") or "", " ".join(p["text"].split()),
                sources_fp(p["sources"]), compact(p["record_type"]), p.get("project") or "",
                compact(p["people"]), p.get("target_updated_at") or ""])


def main():
    state = Path(sys.argv[1]).resolve()
    flags = set(sys.argv[2:])
    if state == ROOT or ROOT in state.parents:
        raise SystemExit("Fixture output must be outside the source checkout")
    today = date(2026, 11, 8)
    day = today.isoformat()
    previous = (today - timedelta(days=1)).isoformat()
    stamp = f"{day}T10:00:00Z"

    def save(name, value):
        path = state / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps(value, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")

    cite = {"type": "meeting", "reference": f"[[{MEETING}@14:22]]"}
    kb = {"revision": "0" * 40, "captured_at": stamp,
          "path": str(state / "kb-snapshot" / "synthetic"), "stale": False}
    summary = json.loads((ROOT / "tools/ui-fixtures/meetings" / MEETING / "english-summary.json").read_text())
    findings = []
    kinds = ["other", "project_mismatch", "task_without_owner", "overdue_or_stale", "missing_followup",
             "conflicting_decisions", "story_gap", "decision_not_in_kb", "duplicate_task", "other"]
    titles = ["Review the synthetic workshop", "Choose the Atlas project", "Name a demo owner", "Confirm the prototype date",
              "Record the review outcome", "Two demo dates need a decision", "Check the sample export",
              "Copy the decision to the register", "Review two sample tasks",
              "Choose the next review date"]
    actions = [("set_project", "Northwind Atlas"), ("set_owner", "Morgan"),
               ("set_due", "2026-11-12"), ("mark_done", True)]
    for index, (kind, title) in enumerate(zip(kinds, titles)):
        item = {"id": f"finding-synthetic-{index}", "kind": kind, "severity": "medium",
                "title": title, "explanation": f"Synthetic Northwind example {index + 1}. Alex and Morgan need to confirm this item.",
                "project": "Northwind Atlas", "evidence": [cite["reference"]],
                "subject_refs": [f"task:{MEETING}:{index % len(summary['todo'])}"],
                "subject_keys": [f"synthetic-{index}"], "material_state": f"synthetic-{index}",
                "evidence_keys": [cite["reference"]], "first_seen": previous,
                "last_seen": day, "change": "new" if index == 0 else "changed" if index == 1 else "still_open"}
        if "--actions" in flags and 1 <= index <= 4:
            task_index = index - 1
            kind_action, value = actions[task_index]
            item["action"] = {"kind": kind_action, "meeting_id": MEETING, "todo_index": task_index,
                              "expected_task": summary["todo"][task_index]["task"], "value": value}
        findings.append(item)

    records = []
    for index in range(30):
        status = "retired" if index == 20 else "superseded" if index == 21 else "forgotten" if index == 25 else "active"
        kind = ["person", "project", "decision", "commitment", "question", "story"][index % 6]
        record_source = {"type": "meeting", "reference": f"[[{MEETING}@00:{index:02}]]"}
        record = {"id": f"memory-synthetic-{index}", "type": kind,
                  "text": f"Northwind Atlas sample {index + 1}: Morgan will review the paper prototype.",
                  "project": "Northwind Atlas", "people": ["Morgan"], "sources": [record_source],
                  "first_seen": previous, "last_confirmed": None, "status": status,
                  "basis": "source_supported", "supersedes": None, "updated_at": stamp,
                  "source_fingerprint": sources_fp([record_source]), "source_fingerprints": [sources_fp([record_source])]}
        if status == "forgotten":
            record.pop("text")
            record.update(project=None, people=[], sources=[])
        records.append(record)
    proposals = []
    if "--proposals" in flags:
        for op, target in [("add", None), ("confirm", records[0]), ("supersede", records[4]), ("retire", records[23])]:
            p = {"op": op, "target_id": target["id"] if target else None,
                 "text": target["text"] if target else "Northwind Atlas will use a paper prototype for the demo.",
                 "record_type": target["type"] if target else "decision", "project": "Northwind Atlas",
                 "people": target["people"] if target else [], "sources": [cite],
                 "rationale": "Synthetic proposal for owner review.", "target_updated_at": target["updated_at"] if target else None}
            p["id"] = proposal_id(p)
            proposals.append(p)
    legacy = f"# Northwind Atlas\n\nMorgan will review the paper prototype. [[{MEETING}@14:22]]\n"
    path = state / "memory/company.md"
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(legacy, encoding="utf-8", newline="\n")
    response = {"paragraphs": [{"id": 0, "records": [{
        "type": "commitment", "text": "Morgan will review the paper prototype.",
        "project": None, "people": ["Morgan"], "sources": [cite["reference"]],
        "evidence": "Morgan will review the paper prototype.", "first_seen": None,
    }], "unplaced": None}]}
    save("local/memory/migration-preview.json", {
        "version": 1, "source_fingerprint": fnv([legacy]), "response": compact(response),
        "error": None, "runner": "Synthetic cached response (no model call)", "runtime_ms": 0,
    })
    if "--memory" in flags:
        save("memory/records.json", {"schema_version": 1, "records": records, "proposals": [], "events": [],
                                     "migration": {"version": 1, "source_path": "synthetic", "fingerprint": fnv([legacy]), "accepted_at": stamp}})
        (state / "memory/company.legacy.md").write_text(legacy, encoding="utf-8", newline="\n")
    report = {"schema_version": 2 if "--v2" in flags else 1, "date": day,
              "runner": "Synthetic fixture (no model call)", "prompt_bytes": 0, "elapsed_seconds": 0,
              "meeting_count": 4, "task_count": 20, "kb": kb, "warnings": [], "memory_updated": False,
              "findings": findings, "brief": ["One new synthetic finding.", "The demo date needs a decision.", "Prototype review is next."],
              "coverage": {"meeting_count": 4, "task_count": 20, "kb_revision": kb["revision"], "kb_captured_at": stamp, "warnings": []},
              "story_alignment": [{"project": "Northwind Atlas", "status": "tension",
                                   "kb_claim": {"text": "The demo is on Thursday.", "sources": ["projects/atlas.md"]},
                                   "meeting_claim": {"text": "The demo may move to Friday.", "sources": [cite["reference"]]},
                                   "question": "Which date should the sample register use?"}],
              "memory_proposals": proposals,
              "comparison": {"baseline_date": previous, "complete": True, "new": [findings[0]["id"]],
                             "changed": [findings[1]["id"]], "carried": [f["id"] for f in findings[2:]],
                             "resolved": ["finding-synthetic-resolved"], "unmatched": ["finding-synthetic-unmatched"]},
              "resolutions": [{"finding_id": "finding-synthetic-resolved", "explanation": "The sample review finished.", "evidence": [cite["reference"]]}]}
    save(f"checks/{day}.json", report)
    if flags & {"--earlier", "--gaps", "--v2"}:
        for back in ([1, 3, 4] if "--gaps" in flags else [1]):
            old = (today - timedelta(days=back)).isoformat()
            save(f"checks/{old}.json", dict(report, date=old, memory_proposals=[]))
    if "--triage" in flags:
        entries = {}
        for index, status in [(9, "dismissed"), (7, "snoozed")]:
            f = findings[index]
            entries[f["id"]] = {"finding_id": f["id"], "status": status, "updated_at": stamp,
                                "snooze_until": "2026-11-11" if status == "snoozed" else None,
                                "evidence_fingerprint": fnv(f["evidence_keys"]), "evidence_keys": f["evidence_keys"], "note": None}
        save("checks/triage.json", {"schema_version": 1, "findings": entries})
    (state / "FIXTURE.txt").write_text("Wholly invented Northwind render data.\n", encoding="utf-8")


if __name__ == "__main__":
    main()
