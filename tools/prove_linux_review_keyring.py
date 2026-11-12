#!/usr/bin/env python3
"""Two concurrent config roots on one real Secret Service; print no secret values."""
import hashlib
import os
from pathlib import Path
import subprocess
import sys
import tempfile
from gi.repository import Gio, GLib

example = Path(sys.argv[1])
root = Path(tempfile.mkdtemp(prefix="namespaces-", dir=sys.argv[2]))
cases = []
for name in ("left", "right"):
    case = root / name
    case.mkdir(mode=0o700)
    config = case / "config"
    config.mkdir(mode=0o700)
    env = os.environ.copy()
    env.pop("CREDENTIALS_DIRECTORY", None)
    env["XDG_CONFIG_HOME"] = str(config)
    process = subprocess.Popen([str(example), "identity", str(case / "state.sqlite")],
                               env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    cases.append((case, config, env, process))
fingerprints = []
items = []
connection = Gio.bus_get_sync(Gio.BusType.SESSION, None)
for case, config, env, process in cases:
    output, error = process.communicate(timeout=8)
    assert process.returncode == 0, error
    again = subprocess.check_output([str(example), "identity", str(case / "state.sqlite")], env=env, text=True)
    assert again == output
    fingerprints.append(output)
    install = hashlib.sha256(os.fsencode(config / "sidenote/secrets")).hexdigest()
    result = connection.call_sync("org.freedesktop.secrets", "/org/freedesktop/secrets",
                                  "org.freedesktop.Secret.Service", "SearchItems",
                                  GLib.Variant("(a{ss})", ({"application": "sidenote", "install": install},)),
                                  None, Gio.DBusCallFlags.NONE, 2000, None).unpack()
    assert len(result[0]) == 1 and not result[1], result
    items.extend(result[0])
assert len(set(items)) == 2
assert fingerprints[0] != fingerprints[1]
print("concurrent XDG roots: two distinct keyring items; both identities reload: PASS")
