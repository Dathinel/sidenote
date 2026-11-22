"""Classify the tree GitHub checked out."""
import json
import os
from pathlib import Path, PurePosixPath
import re
import subprocess
import sys

WINDOWS_CFG = re.compile(rb'\bcfg(?:_attr)?!?\s*\([^\]]*\bwindows\b')
WINDOWS_PATH = re.compile(r'(?:^|/|_)windows(?:[./_]|$)', re.IGNORECASE)
WINDOWS_ONLY = {
    "assets/fonts/Figtree-Variable.ttf", "assets/icon/meetings.ico", "build.ps1", "src/core/background_tests.rs",
    "src/core/meeting_evidence/speech_host_tests.rs", "src/core/meeting_router/jobs/handoff/tests.rs",
    "src/core/meeting_router/jobs/r1_tests.rs", "src/core/meeting_router/revision/captured_tests.rs",
    "src/core/meeting_router/revision/receipts.rs", "src/hex_install.rs", "src/hex_probe.rs",
    "src/install_hooks_tests.rs", "src/meeting_icon.rs", "tests/idle_cpu.rs",
}
# These files are compiled only as a named test module in the root library.
MODULES = {
    "src/core/annotations/notebook_tests.rs": "core::annotations::notebook_tests::",
    "src/sync_owner/read_tests.rs": "sync_owner::read_tests::",
}
DOCS = {"LICENSE", "AGENTS.md", "README.md", "CHANGELOG.md", "CONTRIBUTING.md", "SECURITY.md", "THIRD_PARTY.md"}

def git(*args):
    return subprocess.check_output(["git", *args])

def classify(paths, main, windows_cfg):
    docs = {p for p in paths if p in DOCS or p.startswith(("docs/", "reviews/", "artifacts/"))}
    relay = {p for p in paths if p.startswith("relay/")}
    tools = {p for p in paths if p.startswith(("tools/", "scripts/")) or p == "tests/linux_package.py"}
    workflow_paths = {p for p in paths if p.startswith(".github/")}
    workflow = bool(workflow_paths)
    embedded = {p for p in tools if p.startswith(("scripts/meeting-pipeline/", "tools/ui-fixtures/"))}
    rust = (paths - docs - relay - workflow_paths - tools) | embedded
    # Only a test-only module can safely avoid the whole suite. Tools may embed test fixtures.
    focused = MODULES.get(next(iter(rust)), "") if len(rust) == 1 and not (main or workflow) else ""
    windows = any(
        p in WINDOWS_ONLY or p.startswith(("src/meeting_icon/", "src/core/meeting_router/production/",
                                           ".cargo/", "scripts/meeting-pipeline/"))
        or PurePosixPath(p).name in {"Cargo.toml", "Cargo.lock", "rust-toolchain.toml", "build.rs"}
        or (p.endswith(".rs") and (WINDOWS_PATH.search(p) or windows_cfg(p)))
        for p in rust
    )
    return {"docs_only": paths == docs, "rust": bool(rust), "workflow": workflow,
            "linux": main or workflow or bool(rust), "windows": main or workflow or windows,
            "relay": bool(relay), "tools": bool(tools), "module": focused}

def changed_jobs():
    event = json.loads(Path(os.environ["GITHUB_EVENT_PATH"]).read_text(encoding="utf-8"))
    main = os.environ["GITHUB_EVENT_NAME"] == "push"
    before = event["before"] if main else git("rev-parse", "HEAD^1").decode().strip()
    tracked = git("ls-tree", "-r", "--name-only", "-z", "HEAD").decode().split("\0")
    if main and (not before.strip("0") or subprocess.run(
        ["git", "cat-file", "-e", before + "^{commit}"], capture_output=True
    ).returncode):
        paths = set(tracked) - {""}
        old = set()
    else:
        paths = set(git("diff", "--no-renames", "--name-only", "-z", before, "HEAD").decode().split("\0")) - {""}
        old = set(git("ls-tree", "-r", "--name-only", "-z", before).decode().split("\0"))
    current = set(tracked)

    def windows_cfg(path):
        # A deleted Windows gate matters as much as an added one.
        return ((path in old and WINDOWS_CFG.search(git("show", f"{before}:{path}")))
                or (path in current and WINDOWS_CFG.search(git("show", f"HEAD:{path}"))))

    outputs = classify(paths, main, windows_cfg)
    outputs["windows"] = outputs["windows"] and os.environ["HOSTED_WINDOWS"] == "on"
    print(json.dumps(outputs))
    with Path(os.environ["GITHUB_OUTPUT"]).open("a", encoding="utf-8") as output:
        for key, value in outputs.items():
            print(f"{key}={str(value).lower() if isinstance(value, bool) else value}", file=output)


def self_test():
    def cfg(path):
        return path == "src/mixed.rs"
    assert classify({"docs/design.md"}, False, cfg)["docs_only"]
    assert not classify({"relay/index.ts"}, False, cfg)["linux"]
    assert classify({"relay/index.ts"}, False, cfg)["relay"]
    assert classify({"tools/pipeline-tests.py"}, False, cfg)["tools"]
    assert not classify({"tools/pipeline-tests.py"}, False, cfg)["linux"]
    assert classify({"tools/ui-fixtures/hex.json"}, False, cfg)["linux"]
    assert not classify({"src/lib.rs"}, False, cfg)["windows"]
    assert classify({"src/mixed.rs"}, False, cfg)["windows"]
    assert classify({"src/platform/windows/job_process.rs"}, False, cfg)["windows"]
    assert classify({"src/hex_install.rs"}, False, cfg)["windows"]
    assert classify({".github/workflows/ci.yml"}, False, cfg)["linux"]
    assert classify({"README.md"}, True, cfg)["windows"]
    module, name = next(iter(MODULES.items()))
    assert classify({module, "README.md"}, False, cfg)["module"] == name
    assert classify({module, "Cargo.toml"}, False, cfg)["module"] == ""
    assert WINDOWS_CFG.search(b'#[cfg(all(\n  test, target_os = "windows"\n))]')
    print("CI classifier self-test passed")

if __name__ == "__main__":
    if sys.argv[1:] == ["--self-test"]:
        self_test()
    else:
        changed_jobs()
