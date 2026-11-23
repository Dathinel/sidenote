#!/usr/bin/env python3
"""Cheap adoption guard: reviewed call-site inventory plus mandatory Linux boundaries."""
import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess
import sys
from tempfile import TemporaryDirectory

SPAWN = re.compile(r"Command::new|(?:\.|::|\b)(?:spawn|exec[vl]?[ep]?|fork|vfork|posix_spawnp?|CreateProcessW)\s*\(|\.output\s*\(|\.status\s*\(")
READ = re.compile(r"read_bounded|store_io::read|File::open|(?:std::)?fs::read(?:_to_string)?\s*\(|file_reads::(?:read|text|nofollow)|Snapshot::(?:capture|read_file|open_file)|shared_read\s*\(")
INVENTORY = Path("tools/linux-boundary-inventory.json")


def scan_inventory(root, source):
    actual = {}
    for directory in ("src", "crates"):
        for path in (root / directory).rglob("*.rs"):
            name = path.relative_to(root).as_posix()
            text = source(name)
            if SPAWN.search(text) or READ.search(text):
                actual[name] = hashlib.sha256(text.encode()).hexdigest()
    return actual


def unique_entries(pairs):
    entries = {}
    for name, digest in pairs:
        assert name not in entries, f"duplicate boundary inventory key: {name}"
        entries[name] = digest
    return entries


def update(root):
    actual = scan_inventory(root, lambda name: (root / name).read_text(encoding="utf-8"))
    data = json.dumps(actual, indent=2, sort_keys=True).replace(",\n", ",\n\n") + "\n"
    path = root / INVENTORY
    with TemporaryDirectory(dir=path.parent) as directory:
        staged = Path(directory) / path.name
        staged.write_bytes(data.encode("utf-8"))
        staged.replace(path)
    print(f"Updated Linux boundary inventory ({len(actual)} files)")


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
    expected = json.loads((root / INVENTORY).read_text(), object_pairs_hook=unique_entries)
    actual = scan_inventory(root, source)
    if actual != expected:
        stale = sorted(name for name in actual.keys() | expected.keys() if actual.get(name) != expected.get(name))
        raise AssertionError("stale boundary inventory: " + ", ".join(stale))
    print("PASS Linux decoder disabled; meeting reads pinned; reviewed helper/reader inventory matches")


def test_update(root):
    with TemporaryDirectory() as directory:
        scratch = Path(directory)
        for name in (
            "crates/sidenote-sync/src/client.rs",
            "crates/sidenote-sync/src/worker.rs",
            "src/core/file_reads.rs",
            "src/core/meeting_router/inputs.rs",
            "src/core/notes/queue.rs",
            "src/core/notes/transcript.rs",
        ):
            target = scratch / name
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes((root / name).read_bytes())
        inventory = scratch / INVENTORY
        inventory.parent.mkdir()
        fixture = scratch / "src/fixture.rs"
        text = 'fn example() { let _ = std::process::Command::new("fixture"); }\n// A\n'
        fixture.write_text(text, encoding="utf-8")

        def run(*flags, exit_code=0):
            command = [sys.executable, "-I", str(Path(__file__).resolve()), str(scratch), *flags]
            result = subprocess.run(command, capture_output=True, text=True)
            assert result.returncode == exit_code, result.stdout + result.stderr
            return result.stdout

        run("--update")
        original = inventory.read_bytes()
        parsed = json.loads(original)
        assert parsed["src/fixture.rs"] == hashlib.sha256(text.encode()).hexdigest()
        assert list(parsed) == sorted(parsed), "inventory keys must be sorted"
        assert original.count(b",\n\n") == len(parsed) - 1, "inventory entries must have one blank separator"
        assert original.endswith(b"\n"), "inventory must end in a newline"
        run("--update")
        assert inventory.read_bytes() == original, "unchanged inputs must reproduce the same bytes"
        run()

        fixture.write_text(text.replace("// A", "// B"), encoding="utf-8")
        assert run(exit_code=1).strip() == "stale boundary inventory: src/fixture.rs"
        run("--update")
        run()

        fixture.write_text("fn example() {}\n", encoding="utf-8")
        (scratch / "src/new.rs").write_text(text, encoding="utf-8")
        assert run(exit_code=1).strip() == "stale boundary inventory: src/fixture.rs, src/new.rs"
        run("--update")
        run()

        current = inventory.read_text(encoding="utf-8")
        digest = hashlib.sha256(text.encode()).hexdigest()
        for stale in ("0" * 64, digest):
            duplicate = f'{{\n  "src/new.rs": "{stale}",\n\n'
            inventory.write_text(duplicate + current[2:], encoding="utf-8")
            assert run(exit_code=1).strip() == "duplicate boundary inventory key: src/new.rs"
    print("PASS update writes stable JSON and refreshes changed, added, and removed entries; duplicate keys fail")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("root", type=Path, nargs="?", default=Path(__file__).resolve().parents[1])
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--self-test", action="store_true")
    mode.add_argument("--update", action="store_true")
    args = parser.parse_args()
    root = args.root
    if args.update:
        update(root)
        return
    verify(root)
    if args.self_test:
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
        test_update(root)


if __name__ == "__main__":
    try:
        main()
    except AssertionError as error:
        print(error)
        sys.exit(1)
