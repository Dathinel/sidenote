"""Verify an ordinary optimized app refuses the qualification driver offscreen."""
import json
from pathlib import Path
import subprocess
import sys
import tempfile


def normal_release_rejects_headless_driver(binary):
    parent = Path(tempfile.gettempdir()) / "sidenote-relay-preview"
    parent.mkdir(exist_ok=True)
    with tempfile.TemporaryDirectory(dir=parent) as temporary:
        root = Path(temporary)
        args = [str(binary), "--relay-preview-headless"]
        for flag, folder in (("--data-dir", root), ("--meetings-dir", root / "meetings"), ("--peers-dir", root / "peers")):
            folder.mkdir(exist_ok=True)
            args.extend((flag, str(folder)))
        result = subprocess.run(args, capture_output=True, timeout=30, creationflags=subprocess.CREATE_NO_WINDOW)
        assert result.returncode == 2, "Ordinary release accepted the qualification entry point"
        assert not result.stdout and not result.stderr, "Refusal must produce no protocol or diagnostic output"
        assert not (root / "local/relay/replica.sqlite").exists(), "Refusal opened a replica"


if __name__ == "__main__":
    binary = Path(sys.argv[1]).resolve(strict=True)
    normal_release_rejects_headless_driver(binary)
    print(json.dumps({"test": "normal_release_rejects_headless_driver", "passed": True}))
