#!/usr/bin/env python3
"""Test the packaged system unit without changing another lane's unit or process."""
import json
import os
from pathlib import Path
import shutil
import socket
import subprocess
import sys
import time

from prove_linux_step3 import wait
from prove_linux_boundaries import verify as verify_boundaries


def command(*args, accepted=(0,)):
    result = subprocess.run(args, capture_output=True, timeout=30)
    if b"helper reached host abstract-address socket" in result.stderr:
        raise AssertionError("helper reached host abstract-address socket")
    assert result.returncode in accepted, "boot proof command failed: " + args[0]
    return result.stdout.decode().strip()


def main(binary):
    verify_boundaries(Path.cwd())
    binary = str(Path(binary).resolve())
    uid = str(os.getuid())
    abstract = socket.socket(socket.AF_UNIX)
    tcp = socket.socket()
    abstract_address = "/tmp/dbus-sidenote-step5-proof-" + uid
    abstract.bind("\0" + abstract_address)
    abstract.listen(1)
    tcp.bind(("127.0.0.1", 0))
    tcp.listen(1)
    host_network = [abstract_address, tcp.getsockname()[1],
                    os.environ.get("DBUS_SESSION_BUS_ADDRESS", ""), os.readlink("/proc/self/ns/net")]
    unit = "sidenote-step5-boot@" + uid + ".service"
    worker = "sidenote-step5-boot-worker.service"
    target = Path("/run/systemd/system/sidenote-step5-boot@.service")
    root = Path("/var/lib/sidenote-step5-proof-" + uid)
    assert not target.exists() and not root.exists(), "refuse existing proof state"
    executable = Path("/run/sidenote-step5-proof-" + uid)
    assert not executable.exists(), "refuse existing proof binary directory"
    command("sudo", "-n", "install", "-d", "-m", "0755", str(executable))
    command("sudo", "-n", "install", "-m", "0755", binary, str(executable / "sidenote"))
    command("sudo", "-n", "install", "-m", "0755", str(Path(binary).with_name("sidenote-linux-worker")),
            str(executable / "sidenote-linux-worker"))
    source = Path("packaging/systemd/sidenote-server@.service").read_text()
    source = source.replace("sidenote-server-worker", "sidenote-step5-boot-worker")
    source = source.replace("sidenote-%i", "sidenote-step5-proof-%i")
    source = source.replace("/usr/local/libexec/sidenote/sidenote", str(executable / "sidenote"))
    source = source.replace("sidenote-key:/etc/credstore.encrypted/sidenote-step5-proof-%i",
                            "sidenote-key:/home/boxes/linux-step2-proof/sidenote-key.cred")
    source = source.replace("/etc/group\"", "/etc/group " + str(executable) + "\"")
    source = source.replace("--server --device-role server", "--server --device-role server --mcp-port 0")
    staged = Path("/home/boxes/linux-step5-receipts/boot-proof.service")
    staged.write_text(source)
    staged.chmod(0o600)
    command("sudo","-n","install","-m","0644",str(staged),str(target))
    paths = ["--data-dir",str(root / "state"),"--meetings-dir",str(root / "meetings"),
             "--peers-dir",str(root / "peers")]
    try:
        denied = subprocess.run(["sudo", "-n", binary, "--server", *paths],
                                capture_output=True, timeout=15)
        assert denied.returncode == 2 and b"SERVICE_ROOT_REFUSED" in denied.stderr
        assert not root.exists(), "root refusal must precede service state creation"
        assert "ExecStartPre=/usr/bin/test %i -ne 0" in source
        guard = "sidenote-step5-root-refusal-" + uid
        denied = subprocess.run(["sudo", "-n", "systemd-run", "--unit=" + guard,
                                 "--wait", "--pipe", "--collect",
                                 "--property=ExecStartPre=/usr/bin/test 0 -ne 0",
                                 "/usr/bin/touch", str(executable / "root-started")],
                                capture_output=True, timeout=30)
        assert denied.returncode != 0 and not (executable / "root-started").exists()
        print("PASS binary refuses root before state creation; unit root guard rejects uid 0")
        command("sudo","-n","systemctl","daemon-reload")
        command("sudo","-n","systemd-analyze","verify",str(target))
        command("sudo","-n","systemctl","enable","--runtime",unit)
        # loginctl fails when the uid has no logind user object: that also means no session.
        sessions = command("loginctl","show-user",uid,"-p","Sessions","--value",accepted=(0,1))
        assert not sessions, "interactive sessions invalidate this proof"
        command("sudo","-n","systemctl","start",unit)
        endpoint = root / "state/local/sync/control.sock"
        try:
            wait(endpoint.exists)
        except AssertionError:
            print(command("journalctl", "--user", "-u", worker, "--no-pager", "-n", "16", "-o", "cat"))
            raise
        pid = int(command("systemctl","--user","show",worker,"-p","MainPID","--value"))
        assert pid > 0
        decoder = subprocess.run([str(Path(binary).with_name("sidenote-sync-worker"))],
                                 stdin=subprocess.DEVNULL, capture_output=True, timeout=5)
        assert decoder.returncode == 2 and decoder.stdout == b""
        assert decoder.stderr == b"Relay sync is not yet available on Linux\n"
        settings = root / "state/local/relay/settings.json"
        settings.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
        settings.write_text('{"enabled":true}')
        settings.chmod(0o600)
        unavailable = json.loads(command(binary, *paths, "--ops", "status"))
        assert unavailable["peers"]["transport"] == "unavailable"
        group = command("systemctl", "--user", "show", worker, "-p", "ControlGroup", "--value")
        processes = Path("/sys/fs/cgroup" + group + "/cgroup.procs").read_text().split()
        assert set(processes) == {str(pid)}, "unavailable relay launched a child process"
        print("PASS Linux relay decoder entry refused before IPC")
        print("PASS relay status unavailable; stored opt-in launches no decoder")
        expected = {"UMask": "0077", "PrivateUsers": "yes", "ProtectSystem": "strict", "ProtectHome": "tmpfs",
                    "PrivateTmp": "yes", "NoNewPrivileges": "yes", "PrivateDevices": "yes",
                    "LockPersonality": "yes", "RestrictRealtime": "yes", "RestrictSUIDSGID": "yes",
                    "ProtectKernelTunables": "yes", "ProtectKernelModules": "yes",
                    "ProtectControlGroups": "yes", "MemoryMax":"536870912", "TasksMax":"32", "OOMPolicy":"stop", "RootDirectory": str(root), "MountAPIVFS": "yes"}
        properties = command("systemctl", "--user", "show", worker,
                             *["--property=" + name for name in
                               [*expected, "ReadWritePaths", "BindReadOnlyPaths", "RestrictAddressFamilies", "SystemCallFilter", "MemoryMax", "TasksMax", "UnsetEnvironment", "OOMPolicy"]])
        live = dict(line.split("=", 1) for line in properties.splitlines())
        print("LIVE USER WORKER PROPERTIES\n" + properties)
        assert all(live.get(name) == value for name, value in expected.items()), live
        assert {path.removeprefix("+") for path in live["ReadWritePaths"].split()} == {"/state", "/meetings", "/peers"}
        assert {binding.split(":")[0] for binding in live["BindReadOnlyPaths"].split()} == {"/usr", "/lib", "/lib64", "/etc/ssl", "/etc/resolv.conf", "/etc/hosts", "/etc/nsswitch.conf", "/etc/passwd", "/etc/group", str(executable)}
        assert set(live["RestrictAddressFamilies"].split()) == {"AF_UNIX", "AF_INET", "AF_INET6"}
        mount_calls = {line.strip() for line in command("systemd-analyze", "syscall-filter", "@mount").splitlines()
                       if line.strip() and not line.lstrip().startswith(("#", "@"))}
        assert live["SystemCallFilter"].startswith("~")
        assert set(live["SystemCallFilter"][1:].split()) == mount_calls
        assert command("sudo", "-n", "readlink", f"/proc/{pid}/ns/user") != os.readlink("/proc/self/ns/user")
        status = Path(f"/proc/{pid}/status").read_text()
        assert "NoNewPrivs:\t1" in status
        for line in status.splitlines():
            if line.startswith(("CapEff:", "CapPrm:", "CapInh:")): assert int(line.split()[1], 16) == 0
        manager = command("systemctl", "show", "user@" + uid + ".service", "-p", "MainPID", "--value")
        assert manager.isdigit() and int(manager) > 0
        probe = r'''import os, pathlib, sys, subprocess
root = pathlib.Path(sys.argv[1])
uid = str(os.getuid())
for bus in ["/run/user/" + uid + "/bus", "/proc/" + sys.argv[2] + "/root/run/user/" + uid + "/bus"]:
    env = os.environ.copy()
    env["DBUS_SESSION_BUS_ADDRESS"] = "unix:path=" + bus
    env["XDG_RUNTIME_DIR"] = str(pathlib.Path(bus).parent)
    result = subprocess.run(["/usr/bin/systemd-run", "--user", "--wait", "--pipe", "--collect",
                             "/usr/bin/touch", "/tmp/sidenote-step5-escape-" + uid],
                            env=env, capture_output=True, timeout=5)
    assert result.returncode != 0, "unconstrained user unit started"
print("PASS attempted unconstrained user unit launch denied", flush=True)

(root / "sandbox-write-probe").touch(mode=0o600)
try:
    pathlib.Path("/state-outside-write-probe").touch()
except OSError:
    pass
else:
    raise AssertionError("sandbox allowed an outside write")
assert not pathlib.Path("/home/boxes").exists()
p = pathlib.Path("/tmp/sidenote-step5-private-" + str(os.getuid()))
p.touch(mode=0o600)
print("PASS live mount namespace: own state writable, outside writes denied, home hidden, private tmp writable")
'''
        print(command("sudo", "-n", "nsenter", "--target", str(pid), "--user", "--mount",
                      "--root", "--wd", "--setuid", uid, "--setgid", str(os.getgid()), "/usr/bin/python3", "-c", probe, "/state", manager))
        assert not Path("/tmp/sidenote-step5-private-" + uid).exists()
        helper_probe = probe + r'''
import socket, urllib.parse
abstract_address, port, host_bus, host_net = json.loads(sys.argv[3])
def denied(family, address, label):
    with socket.socket(family) as connection:
        connection.settimeout(1)
        try:
            connection.connect(address)
        except OSError:
            print("PASS helper " + label + " connect denied", flush=True)
        else:
            raise AssertionError("helper reached host " + label)
denied(socket.AF_UNIX, "\0" + abstract_address, "abstract-address socket")
session_addresses = 0
for endpoint in host_bus.split(";"):
    if not endpoint.startswith("unix:"): continue
    fields = dict(part.split("=", 1) for part in endpoint[5:].split(",") if "=" in part)
    if "abstract" in fields:
        session_addresses += 1
        denied(socket.AF_UNIX, "\0" + urllib.parse.unquote(fields["abstract"]), "host session bus")
    elif "path" in fields:
        session_addresses += 1
        denied(socket.AF_UNIX, urllib.parse.unquote(fields["path"]), "host session bus")
denied(socket.AF_INET, ("127.0.0.1", port), "TCP")
assert session_addresses > 0, "host session bus was not probed"
assert os.readlink("/proc/self/ns/net") != host_net, "helper retained host networking"
assert os.environ["SIDENOTE_SANDBOX_CANARY"] == "synthetic-environment-only"
status = pathlib.Path("/proc/self/status").read_text()
for line in status.splitlines():
    if line.startswith(("CapEff:", "CapPrm:", "CapInh:")): assert int(line.split()[1], 16) == 0
assert "NoNewPrivs:\t1" in status
print("PASS direct helper inherits sandbox and live environment; namespace capabilities cleared")
'''
        assert not Path("/tmp/sidenote-step5-escape-" + uid).exists()
        sandbox = ["--property=" + name + "=" + value for name, value in live.items()]
        print(command("systemd-run", "--user", "--unit=sidenote-step5-sandbox-helper",
                      "--service-type=exec", "--wait", "--pipe", "--collect",
                      "--working-directory=/state", *sandbox,
                      "--property=CPUQuota=100%",
                      "--setenv=SIDENOTE_SANDBOX_CANARY=synthetic-environment-only",
                      str(executable / "sidenote-linux-worker"), "--launch",
                      "/usr/bin/python3", "-c", "import json\n" + helper_probe,
                      "/state", manager, json.dumps(host_network)))
        group = Path(f"/proc/{pid}/cgroup").read_text()
        assert "user@" + uid + ".service" in group and worker in group
        maps = Path(f"/proc/{pid}/maps").read_text().lower()
        assert not any(name in maps for name in ("wgpu","libvulkan","libonnxruntime","librune"))
        assert not command("loginctl","show-user",uid,"-p","Sessions","--value",accepted=(0,1))
        assert root.stat().st_mode & 0o777 == 0o700
        for item in (root / "state").rglob("*"):
            if item.is_file(): assert item.stat().st_mode & 0o077 == 0
        assert command("sudo","-n","systemctl","is-enabled",unit) == "enabled-runtime"
        print("PASS packaged system unit enabled for multi-user.target; no interactive session; private state and no GPU/ONNX maps")

        old = pid
        command("systemctl","--user","kill","--kill-whom=main","--signal=SIGKILL",worker)
        def restarted():
            result = command("systemctl","--user","show",worker,"-p","MainPID","--value",accepted=(0,1))
            return result.isdigit() and int(result) > 0 and int(result) != old and endpoint.exists()
        wait(restarted,seconds=60)
        restarts = int(command("sudo","-n","systemctl","show",unit,"-p","NRestarts","--value"))
        assert restarts >= 1
        # MainPID precedes socket rebinding; require the restarted owner's live response.
        def owner_live():
            result = subprocess.run([binary,*paths,"--ops","status"],capture_output=True,timeout=15)
            return result.returncode in (0,1) and json.loads(result.stdout)["live"]
        wait(owner_live, seconds=30)
        print("PASS killed service child restarted through system unit; owner status live; encrypted credential reload succeeded")
        command("sudo","-n","systemctl","stop",unit)
        wait(lambda: not endpoint.exists())
        assert command("systemctl","--user","is-active",worker,accepted=(0,3,4)) != "active"
        print("PASS system-unit stop removes owner endpoint and stops its user worker")
        print("UNVERIFIED physical reboot/power-loss: shared box hosts other lanes; no reboot performed")
    finally:
        abstract.close()
        tcp.close()
        command("sudo","-n","systemctl","disable","--runtime","--now",unit,accepted=(0,1))
        command("systemctl","--user","stop",worker,accepted=(0,5))
        command("sudo","-n","rm","-f",str(target))
        command("sudo","-n","systemctl","daemon-reload")
        command("sudo","-n","systemctl","reset-failed",unit,accepted=(0,1))
        if root.exists():
            assert root == Path("/var/lib/sidenote-step5-proof-" + uid)
            for child in root.iterdir():
                if child.is_dir() and not child.is_symlink(): shutil.rmtree(child)
                else: child.unlink()
            command("sudo","-n","rmdir",str(root))
        command("sudo", "-n", "rm", "-f", str(executable / "sidenote"), str(executable / "sidenote-linux-worker"))
        command("sudo", "-n", "rmdir", str(executable))
        print("PASS owned proof unit, staged binary and synthetic state cleaned")


if __name__ == "__main__":
    main(sys.argv[1])
