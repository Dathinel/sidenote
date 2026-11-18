#!/usr/bin/env python3
"""Isolated box: offline helpers, blocked agent runners and a local git remote."""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time

from prove_linux_step3 import environment, private_json, seed, wait

ID = "1700000000000-1"
CANARY = "step5-synthetic-content-canary"
STUB = r'''#!/usr/bin/python3
from pathlib import Path
(Path(__file__).parent / "calls").touch()
raise SystemExit("agent runner must not launch on Linux")
'''
NETWORK_UNAVAILABLE = "agent runners need network; not yet supported on Linux"


def run(command, env=None, accepted=(0,)):
    result = subprocess.run(command, env=env, capture_output=True, timeout=30)
    assert result.returncode in accepted, "synthetic command failed: " + command[0]
    return result.stdout


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def qualify(binary):
    binary = str(Path(binary).resolve())
    with tempfile.TemporaryDirectory(prefix="sidenote-step5-", dir="/home/boxes") as temp:
        base = Path(temp)
        root = base / "state"
        seed(root, "Synthetic processor", "server")
        env = environment(base, os.environ["CREDENTIALS_DIRECTORY"])
        stub = base / "bin"
        stub.mkdir(mode=0o700)
        (stub / "claude").write_text(STUB)
        (stub / "claude").chmod(0o700)
        env["PATH"] = str(stub) + ":/usr/bin:/bin"
        meetings = base / "meetings"
        peers = base / "peers"
        peers.mkdir(mode=0o700)
        folder = meetings / ID
        private_json(folder / "manifest.json", {"status":"complete", "created_at_ms":1700000000000,
                     "duration_ms":300000,"title":"Synthetic meeting"})
        private_json(folder / "transcript.ndjson", {"text":CANARY + " synthetic" * 30,
                     "start_ms":1000,"end_ms":2000,"source":"microphone"})
        private_json(folder / "quick-title.json", {"title":"Synthetic meeting"})
        config = {"version":1,"notes":{"engine":"app","helper":"claude","auto":True,"enabled_at_ms":1},
                  "pipeline":{"profile":"experimental"},"features":{"daily_check":False}}
        private_json(root / "config.json", config)
        paths = ["--data-dir",str(root),"--meetings-dir",str(meetings),"--peers-dir",str(peers)]
        unit = "sidenote-step5-processing-" + str(os.getpid())
        credential = os.environ["CREDENTIALS_DIRECTORY"] + "/sidenote-key"

        def control(*args, accepted=(0,)):
            return run(["systemctl","--user",*args,unit], accepted=accepted)

        def start():
            command = ["systemd-run","--user","--unit=" + unit,"--service-type=exec","--collect",
                       "--property=UMask=0077","--property=Restart=no","--property=KillMode=control-group",
                       "--property=LoadCredential=sidenote-key:" + credential, "--property=MemoryMax=512M",
                       "--property=TasksMax=32", "--property=CPUQuota=100%", "--property=OOMPolicy=stop"]
            for name in ("PATH","HOME","XDG_DATA_HOME","XDG_CONFIG_HOME","XDG_STATE_HOME","XDG_CACHE_HOME",
                         "XDG_RUNTIME_DIR","DBUS_SESSION_BUS_ADDRESS"):
                if name in env: command.append("--setenv=" + name + "=" + env[name])
            command += [binary,*paths,"--server","--mcp-port","0"]
            run(command)
            wait(lambda: root.joinpath("local/sync/control.sock").exists())

        def stop():
            control("stop", accepted=(0,5))
            wait(lambda: not root.joinpath("local/sync/control.sock").exists())

        job = root / "local/notes-jobs" / (ID + ".json")
        receipt = root / "local/router-queue" / (ID + ".completed")
        summary = folder / "english-summary.json"
        try:
            start()
            wait(lambda: job.exists() and json.loads(job.read_text())["state"] == "no_helper")
            assert json.loads(job.read_text())["message"] == NETWORK_UNAVAILABLE
            assert not stub.joinpath("calls").exists() and not summary.exists()
            wait(lambda: receipt.exists())
            routed = receipt.read_bytes()
            private_json(summary, {"title":"Previously approved synthetic notes","context":[],
                                  "topics":[],"todo":[],"to_settle":[]})
            notes_hash = digest(summary)
            stop()
            start()
            time.sleep(32)
            assert digest(summary) == notes_hash and not stub.joinpath("calls").exists()
            assert receipt.read_bytes() == routed
            print("PASS Linux agent runner fails closed with explicit network message; no CLI launch; approved notes and routing survive restart/ticks")

            # Changed transcript invalidates the automatic routing receipt.
            private_json(folder / "transcript.ndjson", {"text":"changed synthetic " * 30,
                         "start_ms":1000,"end_ms":2000,"source":"microphone"})
            wait(lambda: receipt.read_bytes() != routed)
            assert digest(summary) == notes_hash, "routing change replaced approved notes"
            print("PASS changed input routes again; completed notes remain byte-identical")
            config["pipeline"]["language_mode"] = "forced:hi"
            private_json(root / "config.json",config)
            private_json(root / "meeting-languages.json",["hi"])
            routed_output = root / "local/transcripts" / ID / "transcript.segments.v1.json"
            wait(routed_output.exists)
            previous_output = routed_output.read_bytes()
            failed_attempt = root / "local/router-work" / ID / "attempt.json"
            wait(failed_attempt.exists)
            attempted = failed_attempt.read_bytes()
            time.sleep(5)
            assert failed_attempt.read_bytes() == attempted, "retry backoff ignored"
            assert routed_output.read_bytes() == previous_output
            wait(lambda: failed_attempt.read_bytes() != attempted, seconds=65)
            assert digest(summary) == notes_hash
            print("PASS unavailable inference preserves prior output and retries after backoff; notes survive")


            listing = run([binary,*paths,"--cpu-models","list"], env)
            assert json.loads(listing) and b"onnxruntime-sherpa-v2.0.1-linux-x64-api22 not-loaded" in listing
            result = subprocess.run([binary,*paths,"--cpu-models","verify","indicconformer-hi-ctc"],
                                    env=env,capture_output=True,timeout=15)
            assert result.returncode == 2 and result.stderr == b"CPU_MODEL_OPERATION_FAILED\n"
            print("PASS optional model catalog and missing-files verification; no implicit model download")
            if os.environ.get("STEP5_FETCH_MODEL") == "1":
                selected = min(json.loads(listing), key=lambda row: row["bytes"])
                model_dir = root / "models" / selected["id"]
                receipt_path = model_dir / "installed.json"
                def stored():
                    total = 0
                    if model_dir.exists():
                        for path in model_dir.iterdir():
                            try:
                                if path.is_file(): total += path.stat().st_size
                            except FileNotFoundError:
                                pass  # A completed part may be renamed between directory reads.
                    return total
                fetcher = subprocess.Popen([binary,*paths,"--cpu-models","fetch",selected["id"]],
                                           env=env,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
                try:
                    wait(lambda: stored() > 1_000_000 or fetcher.poll() is not None)
                    assert fetcher.poll() is None and not receipt_path.exists(), "capture an unfinished model transfer"
                finally:
                    if fetcher.poll() is None: fetcher.kill()
                    fetcher.wait(timeout=10)
                assert not receipt_path.exists(), "interrupted transfer was published"
                previous = stored()
                # Exercise explicit resume after interruption/transient network failure.
                # Integrity failures cannot pass: the final CLI re-hashes every file.
                for attempt in range(4):
                    result = subprocess.run([binary,*paths,"--cpu-models","fetch",selected["id"]],
                                            env=env,capture_output=True,timeout=420)
                    if result.returncode == 0: break
                    assert result.returncode == 2 and result.stderr == b"CPU_MODEL_OPERATION_FAILED\n"
                    assert not receipt_path.exists() and stored() >= previous, "failed transfer lost partial progress or published"
                    previous = stored()
                assert result.returncode == 0, "bounded explicit model resumes exhausted"
                run([binary,*paths,"--cpu-models","verify",selected["id"]],env)
                model_file = next(model_dir.glob("*.onnx"))
                metadata = model_file.stat()
                with model_file.open("r+b") as stream:
                    first = stream.read(1)
                    stream.seek(0)
                    stream.write(bytes([first[0] ^ 1]))
                os.utime(model_file, ns=(metadata.st_atime_ns, metadata.st_mtime_ns))
                assert subprocess.run([binary,*paths,"--cpu-models","verify",selected["id"]],
                                      env=env,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL).returncode == 2
                assert all(p.stat().st_mode & 0o077 == 0 for p in model_dir.rglob("*"))
                print("PASS interrupted CPU model fetch and bounded explicit resume; full SHA-256 verification; same-size corruption rejected; runtime unavailable")


            # Local-only KB remote; the check performs the real fetch/archive path.
            kb = base / "kb"
            kb.mkdir(mode=0o700)
            (kb / "docs").mkdir(mode=0o700)
            (kb / "docs/source.md").write_text("Synthetic company facts only.\n")
            for args in (["init","-q","-b","main"],["add","docs"],
                         ["-c","user.name=Synthetic","-c","user.email=synthetic@example.invalid",
                          "commit","-q","-m","Synthetic fixture"]):
                run(["git","-C",str(kb),*args],env)
            remote = base / "kb-origin.git"
            run(["git","clone","-q","--bare",str(kb),str(remote)],env)
            run(["git","-C",str(kb),"remote","add","origin",str(remote)],env)
            config["knowledge_base"] = {"repo_path":str(kb),"revision":"origin/main","source_roots":["docs"]}
            config["features"]["daily_check"] = True
            private_json(root / "config.json",config)
            device = json.loads((root / "device.json").read_text())
            device["daily_check"] = True
            private_json(root / "device.json",device)
            private_json(root / "service-schedule.json",{"enabled":True,"hour":0,"minute":0})
            wait(lambda: root.joinpath("local/scheduled-check.json").exists())
            day = time.strftime("%Y-%m-%d")
            report = root / "local/checks" / (day + ".json")
            attempt = root / "local/scheduled-check.json"
            time.sleep(5)
            assert not json.loads(attempt.read_text())["completed"] and not report.exists()
            assert not stub.joinpath("calls").exists()
            attempt_hash = digest(attempt)
            stop()
            start()
            time.sleep(32)
            assert digest(attempt) == attempt_hash and not report.exists()
            assert not stub.joinpath("calls").exists()
            print("PASS scheduled check remains incomplete while agent runners are unsupported; restart retains retry receipt without launching CLI")

            log = run(["journalctl","--user","-u",unit,"--no-pager","-o","cat"])
            assert CANARY.encode() not in log and b"hex=unavailable" in log
            status = run([binary,*paths,"--ops","status"],env,accepted=(0,1))
            assert CANARY.encode() not in status
            stop()
            assert not root.joinpath("local/mcp/endpoint.json").exists()
            print("PASS redacted no-Hex operation, clean owner/socket shutdown")

            # Hub sees identical inputs and consent but owns no processing.
            device["role"] = "hub"
            private_json(root / "device.json",device)
            prior = {str(p):digest(p) for p in (job,receipt,summary,attempt)}
            start()
            time.sleep(32)
            assert all(digest(Path(p)) == h for p,h in prior.items())
            assert subprocess.run([binary,*paths,"--cpu-models","list"],env=env,
                                  stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL).returncode == 2
            stop()
            print("PASS Hub offline/no-Hex startup, processing disabled, model fetch denied")
        except BaseException:
            if job.exists():
                saved = json.loads(job.read_text())
                print("DIAGNOSTIC notes state=" + str(saved.get("state")) + " attempts=" + str(saved.get("attempts")), flush=True)
            else:
                print("DIAGNOSTIC no notes receipt", flush=True)
            journal = run(["journalctl","--user","-u",unit,"--no-pager","-o","cat"])
            for line in journal.decode().splitlines():
                if line.startswith(("service ","SERVICE_","SYNC_OWNER_","OPS_")):
                    print("DIAGNOSTIC " + line, flush=True)
            raise
        finally:
            control("stop",accepted=(0,5))


if __name__ == "__main__":
    qualify(sys.argv[1])
