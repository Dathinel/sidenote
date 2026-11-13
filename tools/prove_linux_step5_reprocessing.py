#!/usr/bin/env python3
"""Synthetic successful transcript survives failure, timeout, input change and stop."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import wave
from prove_linux_step3 import environment, private_json, seed, wait

ID = "1700000000000-2"

def run(args, accepted=(0,)):
    p = subprocess.run(args, capture_output=True, timeout=30)
    assert p.returncode in accepted, "proof command failed: " + args[0]
    return p.stdout

def main(binary):
    binary = str(Path(binary).resolve())
    with tempfile.TemporaryDirectory(prefix="sidenote-preserve-", dir="/home/boxes") as tmp:
        base = Path(tmp)
        state = base / "state"
        seed(state, "Synthetic preservation", "server")
        env = environment(base, os.environ["CREDENTIALS_DIRECTORY"])
        meetings = base / "meetings"
        folder = meetings / ID
        private_json(folder / "manifest.json", {"status":"complete","duration_ms":30000})
        private_json(folder / "transcript.ndjson", {"text":"synthetic baseline " * 30,
                     "start_ms":0,"end_ms":30000,"source":"system"})
        with wave.open(str(folder / "system.wav"), "wb") as wav:
            wav.setparams((1,2,48000,0,"NONE","not compressed"))
            wav.writeframes(b"\0\0" * 48000)
        peers = base / "peers"
        peers.mkdir(mode=0o700)
        config = {"version":1,"features":{"daily_check":False},"notes":{"engine":"off"},"pipeline":{"profile":"experimental","language_mode":"forced:hi"}}
        private_json(state / "config.json", config)
        private_json(state / "meeting-languages.json", ["hi"])
        dest = state / "local/transcripts" / ID / "transcript.segments.v1.json"
        chunk = {"id":"prior-0","start":0,"end":30,"track":"system","english":"prior successful source"}
        good = {"schema":"transcript.segments.v1","experimental":True,"baseline":[chunk],
                "input_checksum":"1"*64,"segments":[{"start":0,"end":30,"track":"system",
                    "route":"hi","lang":"hi","lang_conf":1.0,"text_original":"distinct successful enrichment",
                    "text_en":"distinct approved translation","english_reading":"distinct approved translation",
                    "provider":"synthetic","model":"synthetic","status":"qualified","sources":[{"chunk":chunk,"joined_start":0,"joined_end":30,"scores":None}],"translate":True}],
                "english_reading":"distinct approved translation","wall_seconds":1,
                "lid_load_seconds":None,"hi_load_seconds":None,"warnings":[]}
        private_json(dest, good)
        original = dest.read_bytes()
        pending = state / "local/router-queue" / (ID + ".json")
        attempt = state / "local/router-work" / ID / "attempt.json"
        active = state / "local/router-work" / ID / "active.json"
        unit = "sidenote-preserve-" + str(os.getpid())
        paths = ["--data-dir",str(state),"--meetings-dir",str(meetings),"--peers-dir",str(peers)]
        stub = base / "normalizer"
        # A real child, forced failure after the parent has pinned and staged its input.
        stub.write_text("#!/usr/bin/python3\nimport time\nfrom pathlib import Path\np=Path(__file__).with_name('block')\nwhile p.exists(): time.sleep(0.05)\nraise SystemExit(1)\n")
        stub.chmod(0o700)
        def start(timeout=10):
            args = ["systemd-run","--user","--unit="+unit,"--service-type=exec","--collect",
                    "--property=UMask=0077","--property=MemoryMax=512M","--property=TasksMax=32",
                    "--property=CPUQuota=100%","--property=OOMPolicy=stop",
                    "--property=LoadCredential=sidenote-key:"+os.environ["CREDENTIALS_DIRECTORY"]+"/sidenote-key"]
            for key in ["HOME","PATH","XDG_CONFIG_HOME","XDG_DATA_HOME","XDG_STATE_HOME","XDG_CACHE_HOME","XDG_RUNTIME_DIR","DBUS_SESSION_BUS_ADDRESS"]:
                if key in env: args.append("--setenv="+key+"="+env[key])
            run(args + [binary,*paths,"--server","--mcp-port","0","--ffmpeg",str(stub),
                        "--router-timeout-seconds",str(timeout)])
            wait(lambda: (state / "local/sync/control.sock").exists())
        def stop():
            run(["systemctl","--user","stop",unit], accepted=(0,5))
            wait(lambda: not (state / "local/sync/control.sock").exists())
        def retained():
            assert dest.read_bytes() == original, "earlier successful transcript overwritten"
            assert pending.exists(), "failed generation lost instead of remaining pending"
            assert not pending.with_suffix(".completed").exists(), "failed revision marked complete"
        def expire():
            if attempt.exists():
                value = json.loads(attempt.read_text()); value["retry_after_ms"] = 0
                private_json(attempt, value)
        try:
            start()
            wait(attempt.exists)
            record = json.loads(attempt.read_text())
            assert record.get("normalization", {}).get("exit_code") == 1, "fixture must execute the failing normalization child: " + json.dumps(record)
            retained()
            generation = pending.read_bytes()
            first = attempt.read_bytes()
            time.sleep(2)
            assert attempt.read_bytes() == first, "retry storm during backoff"
            stop(); expire(); start()
            wait(lambda: attempt.read_bytes() != first)
            retained()
            assert pending.read_bytes() == generation
            print("PASS real child failure preserves distinct success; same generation retries after restart", flush=True)
            stop(); expire()
            (base / "block").touch()
            prior = attempt.read_bytes()
            start(timeout=2)
            wait(lambda: attempt.read_bytes() != prior)
            assert json.loads(attempt.read_text())["child"]["timed_out"]
            retained()
            print("PASS timeout preserves success and pending retry", flush=True)
            stop(); expire(); start()
            wait(active.exists)
            prior = attempt.read_bytes()
            # Replace a source component while a real normalization child is blocked.
            private_json(folder / "transcript.ndjson", {"text":"changed synthetic "*30,"start_ms":0,"end_ms":30000})
            (base / "block").unlink()
            wait(lambda: attempt.read_bytes() != prior)
            retained()
            assert "input" in json.loads(attempt.read_text())["child"]["error"]
            print("PASS unstable source preserves success and pending retry", flush=True)
            stop(); expire(); (base / "block").touch(); start()
            wait(active.exists)
            stop()
            retained()
            print("PASS graceful service stop preserves successful bytes and pending generation", flush=True)
            expire(); (base / "block").unlink()
            prior = attempt.read_bytes()
            start()
            wait(lambda: attempt.read_bytes() != prior)
            retained()
            print("PASS stopped generation retries after restart", flush=True)
        finally:
            stop()

if __name__ == "__main__":
    main(sys.argv[1])
