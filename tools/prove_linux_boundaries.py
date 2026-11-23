#!/usr/bin/env python3
"""Cheap adoption guard: reviewed call-site inventory plus mandatory Linux boundaries."""
from bisect import bisect_right
import hashlib
import json
from pathlib import Path
import re
import sys

SPAWN = re.compile(r"Command::new|(?:\.|::|\b)(?:spawn|exec[vl]?[ep]?|fork|vfork|posix_spawnp?|CreateProcessW)\s*\(|\.output\s*\(|\.status\s*\(")
READ = re.compile(r"read_bounded|store_io::read|File::open|(?:std::)?fs::read(?:_to_string)?\s*\(|file_reads::(?:read|text|nofollow)|Snapshot::(?:capture|read_file|open_file)|shared_read\s*\(")


def boundary_fingerprint(text):
    matches = [match for pattern in (SPAWN, READ) for match in pattern.finditer(text)]
    if not matches:
        return None
    line_starts = [0, *(newline.end() for newline in re.finditer("\n", text))]
    matched_lines = set()
    for match in matches:
        first = bisect_right(line_starts, match.start()) - 1
        end = bisect_right(line_starts, match.end() - 1)
        matched_lines.update(range(first, end))
    lines = text.split("\n")
    # Keep whitespace inside lines because it can be part of a string literal.
    normalized = sorted(lines[index].strip() for index in matched_lines)
    payload = json.dumps([len(matches), normalized])
    return "calls:" + hashlib.sha256(payload.encode()).hexdigest()


def verify(root, overrides=None):
    overrides = overrides or {}
    def source(name):
        return overrides.get(name, (root / name).read_text(encoding="utf-8"))
    worker = source("crates/sidenote-sync/src/worker.rs")
    start = worker.index("pub(crate) fn spawn")
    linux = worker[worker.index('#[cfg(target_os = "linux")]', start):worker.index("#[cfg(not(any(windows", start)]
    assert "RELAY_UNAVAILABLE" in linux and "Command::new" not in linux and ".spawn(" not in linux, "Linux decoder launch must be unavailable"
    client = source("crates/sidenote-sync/src/client.rs")
    start = client[client.index("pub fn start("):client.index("pub fn edit<")]
    assert start.index("RELAY_UNAVAILABLE") < start.index("Pool::adjacent"), "sync start must fail before worker lookup"
    second = start[start.index("pub fn start_with_worker"):]
    assert second.index("RELAY_UNAVAILABLE") < second.index("tokio::spawn"), "sync start must fail before task creation"
    stdio = worker[worker.index("pub fn run_stdio"):]
    assert stdio.index("RELAY_UNAVAILABLE") < stdio.index("suppress_crash_dialogs();"), "decoder entry must reject Linux"
    notes = source("src/core/notes/queue.rs")
    for first, last in [("pub(super) fn manifest", "fn complete"), ("fn text(", "pub(super) fn teams")]:
        body = notes[notes.index(first):notes.index(last, notes.index(first))]
        assert "Snapshot::read_file" in body and "read_bounded" not in body, "notes meeting reads must use Snapshot"
    transcript = source("src/core/notes/transcript.rs")
    assert "Snapshot::read_file" in transcript and "store_io::read_bounded" not in transcript, "notes transcript must use Snapshot"
    reads = source("src/core/file_reads.rs")
    assert "Snapshot::read_file" in reads and "Snapshot::open_file" in reads, "bounded reads must adopt pinned descriptors"
    inputs = source("src/core/meeting_router/inputs.rs")
    assert len(re.findall(r"m\.nlink\(\)\s*!=\s*1", inputs)) >= 2, "input fd must have exactly one link"
    assert "Directory::open(path.parent()" in inputs and "libc::O_NOFOLLOW" in inputs, "all path components must be pinned"
    expected = json.loads((root / "tools" / "linux-boundary-inventory.json").read_text())
    actual = {}
    for directory in ("src", "crates"):
        for path in (root / directory).rglob("*.rs"):
            name = path.relative_to(root).as_posix()
            text = source(name)
            fingerprint = boundary_fingerprint(text)
            if fingerprint is not None:
                actual[name] = fingerprint
    if actual != expected:
        stale = sorted(name for name in actual.keys() | expected.keys() if actual.get(name) != expected.get(name))
        raise AssertionError("stale boundary inventory: " + ", ".join(stale))


def main():
    root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path(__file__).resolve().parents[1]
    verify(root)
    print("PASS Linux decoder disabled; meeting reads pinned; reviewed helper/reader inventory matches")
    if "--self-test" in sys.argv:
        runner = "src/core/runner_process.rs"
        text = (root / runner).read_text(encoding="utf-8")
        verify(root, {runner: text + "\n// Unrelated self-test edit.\n"})
        print("PASS inventoried file accepts an unrelated edit")

        stale_runner = f"stale boundary inventory: {runner}"
        mutations = [
            ("crates/sidenote-sync/src/worker.rs", "anyhow::bail!(crate::RELAY_UNAVAILABLE)",
             'std::process::Command::new("escape").spawn().unwrap(); anyhow::bail!(crate::RELAY_UNAVAILABLE)',
             "Linux decoder launch must be unavailable"),
            ("src/core/notes/queue.rs", "inputs::Snapshot::read_file", "store_io::read_bounded",
             "notes meeting reads must use Snapshot"),
            (runner, "Command::new(program)", 'Command::new("fixture-program")', stale_runner),
            (runner, "let started = Instant::now();",
             "fixture.spawn\n        (); let started = Instant::now();", stale_runner),
            (runner, "Command::new(program)", "fixture_command(program)", stale_runner),
        ]
        for name, old, new, expected_error in mutations:
            text = (root / name).read_text(encoding="utf-8")
            assert old in text
            try:
                verify(root, {name: text.replace(old, new, 1)})
            except AssertionError as error:
                assert str(error) == expected_error, error
            else:
                raise AssertionError(f"boundary guard accepted an injected edit: {name}: {old}")
        print("PASS guard rejects an injected direct spawn and path-based meeting read")
        print("PASS inventory rejects a changed, added (split across lines), or removed spawn match")


if __name__ == "__main__":
    try:
        main()
    except AssertionError as error:
        print(error)
        sys.exit(1)
