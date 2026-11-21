#!/usr/bin/env python3
"""Cheap adoption guard: reviewed call-site inventory plus mandatory Linux boundaries."""
import hashlib
import json
from pathlib import Path
import re
import sys

SPAWN = re.compile(r"Command::new|(?:\.|::|\b)(?:spawn|exec[vl]?[ep]?|fork|vfork|posix_spawnp?|CreateProcessW)\s*\(|\.output\s*\(|\.status\s*\(")
READ = re.compile(r"read_bounded|store_io::read|File::open|(?:std::)?fs::read(?:_to_string)?\s*\(|file_reads::(?:read|text|nofollow)|Snapshot::(?:capture|read_file|open_file)|shared_read\s*\(")


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
            if SPAWN.search(text) or READ.search(text):
                actual[name] = hashlib.sha256(text.encode()).hexdigest()
    if actual != expected:
        stale = sorted(name for name in actual.keys() | expected.keys() if actual.get(name) != expected.get(name))
        raise AssertionError("stale boundary inventory: " + ", ".join(stale))
    print("PASS Linux decoder disabled; meeting reads pinned; reviewed helper/reader inventory matches")


def main():
    root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path(__file__).resolve().parents[1]
    verify(root)
    if "--self-test" in sys.argv:
        mutations = [
            ("crates/sidenote-sync/src/worker.rs", "anyhow::bail!(crate::RELAY_UNAVAILABLE)", "std::process::Command::new(\"escape\").spawn().unwrap(); anyhow::bail!(crate::RELAY_UNAVAILABLE)"),
            ("src/core/notes/queue.rs", "inputs::Snapshot::read_file", "store_io::read_bounded"),
        ]
        for name, old, new in mutations:
            text = (root / name).read_text(encoding="utf-8")
            assert old in text
            try:
                verify(root, {name: text.replace(old, new, 1)})
            except AssertionError:
                continue
            raise AssertionError("boundary guard accepted an injected bypass")
        print("PASS guard rejects an injected direct spawn and path-based meeting read")


if __name__ == "__main__":
    try:
        main()
    except AssertionError as error:
        print(error)
        sys.exit(1)
