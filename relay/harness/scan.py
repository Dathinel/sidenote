"""CI proof scanner. Scan raw bytes before publishing aggregate-only evidence."""
from __future__ import annotations

import json
from collections import Counter

MARKER = b"SIDENOTE-PLAINTEXT-MUST-NEVER-REACH-RELAY"
REQUIRED = {"storage", "in:auth-or-control", "out:control", "in:pair-binary", "out:pair-binary", "in:close", "out:close"}
REQUIRED |= {f"{direction}:envelope-{kind}" for direction in ("in", "out") for kind in range(1, 6)}


def scan(lines: list[str], require_coverage: bool = True) -> dict:
    counts: Counter[str] = Counter()
    bytes_by_kind: Counter[str] = Counter()
    total_bytes = 0
    negatives: Counter[str] = Counter()
    for line in lines:
        offset = line.find('{"event":"relay-proof"')
        if offset < 0:
            continue
        record = json.loads(line[offset:])
        payload = bytes.fromhex(record["bytes"])
        if record["negative"]:
            if record["kind"] not in {"storage", "in:pair-binary"}:
                raise ValueError("unexpected negative-control surface")
            negatives[record["kind"]] += payload.count(MARKER)
            continue
        if MARKER in payload:
            raise ValueError(f"plaintext marker leaked on {record['kind']}")
        counts[record["kind"]] += bool(payload)
        bytes_by_kind[record["kind"]] += len(payload)
        total_bytes += len(payload)
    if not all(negatives[kind] > 0 for kind in ("storage", "in:pair-binary")):
        raise ValueError("SQL and frame negative controls must both be found")
    if require_coverage:
        covered = {kind for kind, size in bytes_by_kind.items() if size > 0}
        if not REQUIRED <= covered:
            raise ValueError(f"missing non-empty capture surfaces: {sorted(REQUIRED-covered)}")
    return {"captures_by_kind": dict(sorted(counts.items())), "bytes_by_kind": dict(sorted(bytes_by_kind.items())), "bytes_scanned": total_bytes,
            "plaintext_marker_occurrences": 0, "negative_control_occurrences": sum(negatives.values()),
            "negative_controls_by_kind": dict(negatives),
            "production_payload_logging": False}
