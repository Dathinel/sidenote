#!/usr/bin/env python3
"""Run on Linux as an unprivileged user, using synthetic state and a provisioned credential."""
import json
import os
from pathlib import Path
import re
import signal
import subprocess
import sys
import time


def run(*args, **kwargs):
    print("$ " + " ".join(map(str, args)), flush=True)
    result = subprocess.run(list(map(str, args)), check=True, text=True,
                            stdout=subprocess.PIPE, stderr=subprocess.STDOUT, **kwargs)
    print(result.stdout, end="", flush=True)
    return result.stdout.strip()


def wait_for(check, seconds=20):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        value = check()
        if value:
            return value
        time.sleep(0.1)
    raise AssertionError("condition timed out")


def main():
    target = Path(sys.argv[1]).resolve()
    root = Path(sys.argv[2]).resolve()
    root.mkdir(mode=0o700, parents=True, exist_ok=True)
    binary = target / "sidenote"
    example = target / "examples/linux_platform_proof"
    fault = target / "examples/worker_fault"
    credentials = Path(os.environ["CREDENTIALS_DIRECTORY"])
    env = os.environ.copy()
    env.pop("DISPLAY", None)
    env.pop("WAYLAND_DISPLAY", None)
    for name in ("HOME", "XDG_CONFIG_HOME", "XDG_DATA_HOME", "XDG_STATE_HOME", "XDG_CACHE_HOME"):
        env[name] = str(root / name.lower())
        Path(env[name]).mkdir(mode=0o700, exist_ok=True)

    bad_credentials = root / "bad-credentials"
    bad_credentials.mkdir(mode=0o700, exist_ok=True)
    bad_key = bad_credentials / "sidenote-key"
    assert not bad_key.exists() and not bad_key.is_symlink()
    fd = os.open(bad_key, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(fd, "wb") as stream:
        stream.write(b"x" * 32)  # Public invalid fixture; never an encryption key.
    os.chmod(bad_key, 0o644)
    for mode in ("world-readable", "symlink", "missing-store"):
        rejected_env = env.copy()
        rejected_env["CREDENTIALS_DIRECTORY"] = str(bad_credentials)
        if mode == "symlink":
            bad_key.unlink()
            bad_key.symlink_to(credentials / "sidenote-key")
        if mode == "missing-store":
            rejected_env.pop("CREDENTIALS_DIRECTORY")
            rejected_env.pop("DBUS_SESSION_BUS_ADDRESS", None)
        result = subprocess.run([str(example), "identity", str(root / f"rejected-{mode}.sqlite")],
                                env=rejected_env, capture_output=True, text=True)
        assert result.returncode != 0 and "SECRET_" in result.stderr
        print(f"identity rejects {mode}: PASS", flush=True)
    bad_key.unlink()

    # Verify the real worker pre-exec adapter outside systemd, including parent SIGKILL.
    parent = subprocess.Popen([str(example), "parent", str(fault)], stdout=subprocess.PIPE, text=True, env=env)
    try:
        child_pid = int(parent.stdout.readline().strip().split("=")[1])
        limits = Path(f"/proc/{child_pid}/limits").read_text()
        assert re.search(r"Max address space\s+536870912\s+536870912", limits)
        assert re.search(r"Max cpu time\s+120\s+120", limits)
        assert os.getpgid(child_pid) == child_pid
        print(f"worker pid={child_pid} address_space=536870912 cpu_seconds=120 process_group=own", flush=True)
        parent.kill()
        parent.wait(timeout=5)

        def killed():
            path = Path(f"/proc/{child_pid}/stat")
            return not path.exists() or path.read_text().split()[2] == "Z"

        wait_for(killed, 5)
        print("parent SIGKILL -> worker killed (outside systemd)", flush=True)
    finally:
        if parent.poll() is None:
            parent.kill()
            parent.wait()

    user_units = Path.home() / ".config/systemd/user"
    user_units.mkdir(mode=0o700, parents=True, exist_ok=True)
    unit_name = "sidenote-step2-proof.service"
    unit = user_units / unit_name
    assert not unit.exists(), "proof must not overwrite an existing unit"
    template = (Path(__file__).resolve().parent.parent / "packaging/systemd/sidenote.service").read_text()
    template = template.replace("ExecStart=%h/.local/bin/sidenote --server --device-role server",
                                f"ExecStart={binary} --server --device-role server --data-dir {root / 'server'}")
    settings = f"LoadCredential=sidenote-key:{credentials / 'sidenote-key'}\n"
    settings += "\n".join(f"Environment={key}={env[key]}" for key in
                          ("HOME", "XDG_CONFIG_HOME", "XDG_DATA_HOME", "XDG_STATE_HOME", "XDG_CACHE_HOME")) + "\n"
    template = template.replace("Type=simple\n", "Type=simple\n" + settings)
    unit.write_text(template)
    try:
        run("systemctl", "--user", "daemon-reload")
        run("systemctl", "--user", "start", unit_name)
        pid = int(run("systemctl", "--user", "show", unit_name, "-p", "MainPID", "--value"))
        wait_for(lambda: (root / "server/local/relay/replica.sqlite").exists())
        time.sleep(3)
        status = Path(f"/proc/{pid}/status").read_text()
        rss = int(re.search(r"^VmRSS:\s+(\d+)", status, re.M)[1])
        assert rss < 100 * 1024
        maps = Path(f"/proc/{pid}/maps").read_text()
        assert not re.search(r"lib(vulkan|GLX|EGL|X11|wayland|nvidia|cuda)|wgpu|librune", maps, re.I)
        print(json.dumps({"role": "server", "idle_rss_kib": rss, "display_gpu_libraries": 0}), flush=True)
        run("systemctl", "--user", "show", unit_name, "-p", "MemoryMax", "-p", "CPUQuotaPerSecUSec", "-p", "KillMode")
        group = run("systemctl", "--user", "show", unit_name, "-p", "ControlGroup", "--value")
        for controller in ("memory.max", "cpu.max"):
            path = Path("/sys/fs/cgroup") / group.lstrip("/") / controller
            print(f"cgroup {controller}={path.read_text().strip() if path.exists() else 'not delegated'}", flush=True)
        run("systemctl", "--user", "stop", unit_name)
        assert run("systemctl", "--user", "show", unit_name, "-p", "ActiveState", "--value") == "inactive"
        db = root / "server/local/relay/replica.sqlite"
        first = run(example, "identity", db, env=env)
        run("systemctl", "--user", "start", unit_name)
        original = int(run("systemctl", "--user", "show", unit_name, "-p", "MainPID", "--value"))
        os.kill(original, signal.SIGKILL)

        def restarted():
            value = subprocess.check_output(["systemctl", "--user", "show", unit_name, "-p", "MainPID", "--value"], text=True).strip()
            return int(value) if value.isdigit() and int(value) not in (0, original) else None

        new_pid = wait_for(restarted)
        time.sleep(2)
        print(f"systemd SIGKILL restart old_pid={original} new_pid={new_pid}", flush=True)
        run("systemctl", "--user", "show", unit_name, "-p", "ActiveState", "-p", "NRestarts")
        run("systemctl", "--user", "stop", unit_name)
        assert run(example, "identity", db, env=env) == first
        print("service-created relay identity survived stop/start/SIGKILL restart", flush=True)
        run("stat", "-c", "%a %U %n", db.parent, db, credentials, credentials / "sidenote-key")
        run("journalctl", "--user", "-u", unit_name, "--no-pager", "-n", "18", "-o", "cat")
        run("loginctl", "show-user", str(os.getuid()), "-p", "Linger")
    finally:
        subprocess.run(["systemctl", "--user", "stop", unit_name], check=False)
        unit.unlink()
        run("systemctl", "--user", "daemon-reload")


if __name__ == "__main__":
    main()
