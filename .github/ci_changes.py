"""Classify the tree GitHub checked out."""
import json
import os
from pathlib import Path
import subprocess
import sys

DOCS = {"LICENSE", "AGENTS.md", "README.md", "CHANGELOG.md", "CONTRIBUTING.md", "SECURITY.md", "THIRD_PARTY.md"}
RUST_ROOT_FILES = {"Cargo.toml", "Cargo.lock", "rust-toolchain.toml", "build.rs", "build.ps1", "default-runners.json"}
RUST_DIRS = (".cargo/", "src/", "crates/", "examples/", "tests/", "assets/", "packaging/")

def git(*args):
    return subprocess.check_output(["git", *args])

def classify(paths, main):
    docs = {p for p in paths if p in DOCS or p.startswith(("docs/", "reviews/", "artifacts/"))}
    relay = {p for p in paths if p.startswith("relay/")}
    tools = {p for p in paths if p.startswith(("tools/", "scripts/")) or p == "tests/linux_package.py"}
    workflow = any(p.startswith(".github/") for p in paths)
    embedded = {p for p in tools if p.startswith(("scripts/meeting-pipeline/", "tools/ui-fixtures/"))}
    rust = {p for p in paths - tools if p in RUST_ROOT_FILES or p.startswith(RUST_DIRS)} | embedded
    run_all = workflow or bool(paths - (docs | relay | tools | rust))
    run_rust = main or run_all or bool(rust)
    return {"docs_only": paths == docs, "rust": bool(rust), "workflow": workflow,
            "linux": run_rust, "windows": run_rust,
            "relay": run_all or bool(relay), "tools": run_all or bool(tools)}

def changed_jobs():
    event = json.loads(Path(os.environ["GITHUB_EVENT_PATH"]).read_text(encoding="utf-8"))
    main = os.environ["GITHUB_EVENT_NAME"] == "push"
    before = event["before"] if main else git("rev-parse", "HEAD^1").decode().strip()
    if main and (not before.strip("0") or subprocess.run(
        ["git", "cat-file", "-e", before + "^{commit}"], capture_output=True
    ).returncode):
        changed = git("ls-tree", "-r", "--name-only", "-z", "HEAD")
    else:
        changed = git("diff", "--no-renames", "--name-only", "-z", before, "HEAD")

    outputs = classify(set(changed.decode().split("\0")) - {""}, main)
    print(json.dumps(outputs))
    with Path(os.environ["GITHUB_OUTPUT"]).open("a", encoding="utf-8") as output:
        for key, value in outputs.items():
            print(f"{key}={str(value).lower() if isinstance(value, bool) else value}", file=output)


def self_test():
    assert classify({"docs/design.md"}, False)["docs_only"]
    relay = classify({"relay/index.ts"}, False)
    assert relay["relay"] and not (relay["linux"] or relay["windows"] or relay["tools"])
    tools = classify({"tools/pipeline-tests.py"}, False)
    assert tools["tools"] and not (tools["linux"] or tools["windows"] or tools["relay"])
    embedded = classify({"tools/ui-fixtures/hex.json"}, False)
    assert embedded["linux"] and embedded["windows"] and embedded["tools"]
    main = classify({"README.md"}, True)
    assert main["linux"] and main["windows"]
    for path in ("src/core/speech_host.rs", "src/lib.rs", "Cargo.lock"):
        result = classify({path}, False)
        assert result["linux"] and result["windows"], result
        assert not (result["relay"] or result["tools"]), result
    for path in (".github/workflows/ci.yml", "ci-support/check.py"):
        result = classify({path}, False)
        assert all(result[job] for job in ("linux", "windows", "relay", "tools")), result
    print("CI classifier self-test passed")

if __name__ == "__main__":
    if sys.argv[1:] == ["--self-test"]:
        self_test()
    else:
        changed_jobs()
