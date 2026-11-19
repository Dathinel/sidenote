#!/usr/bin/env python3
"""Cargo runner for linux_test_env.sh; only the root unit library needs a service."""
import os
from pathlib import Path
import re
import subprocess
import sys
import time

root = Path(sys.argv[1])
args = sys.argv[2:]
inner = args[0] == '--inner'
if inner:
    args = args[1:]
binary = str(Path(args[0]).resolve())
env = os.environ.copy()
env.update(
    TMPDIR=str(root),
    SIDENOTE_DATA_DIR=str(root / 'state'),
    SIDENOTE_HEX_DIR=str(root / 'meetings'),
    SIDENOTE_PEERS_DIR=str(root / 'peers'),
    XDG_CONFIG_HOME=str(root / 'config'),
    XDG_DATA_HOME=str(root / 'data'),
    XDG_CACHE_HOME=str(root / 'cache'),
    XDG_STATE_HOME=str(root / 'xdg-state'),
    CREDENTIALS_DIRECTORY=str(root / 'public-test-key'),
    XDG_RUNTIME_DIR=f'/run/user/{os.getuid()}',
    DBUS_SESSION_BUS_ADDRESS=f'unix:path=/run/user/{os.getuid()}/bus',
)
for name in ('DISPLAY', 'WAYLAND_DISPLAY'):
    env.pop(name, None)
os.umask(0o077)
# Both the desktop binary and root library are named sidenote-<hash>.
is_library = False
if re.fullmatch(r'sidenote-[0-9a-f]+', Path(binary).name):
    listing = subprocess.check_output([binary, '--list'], env=env, text=True)
    is_library = 'test_paths_never_resolve_into_owner_appdata: test' in listing.splitlines()
if is_library and not inner:
    command = [
        'systemd-run', '--user', '--wait', '--pipe', '--collect',
        f'--unit=sidenote-linux-tests-{time.time_ns()}',
        '--property=MemoryMax=512M', '--property=TasksMax=32', '--property=CPUQuota=100%',
        f'--working-directory={Path.cwd()}',
        '/usr/bin/python3', str(Path(__file__).resolve()), str(root), '--inner', *args,
    ]
    sys.exit(subprocess.run(command, env=env).returncode)
if is_library:
    # This library's cfg(test) home supplies its isolated configuration fallback.
    env.pop('XDG_CONFIG_HOME', None)
os.execvpe(binary, args, env)
