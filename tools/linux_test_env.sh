#!/usr/bin/env bash
# Run native Linux tests with synthetic storage and the helper's required cgroup budget.
set -euo pipefail
repo=$(realpath "$(dirname "$0")/..")
if (( $# == 0 )); then
    echo "Usage: bash tools/linux_test_env.sh cargo test [arguments]" >&2
    exit 2
fi
umask 0077
root=$(mktemp -d "${TMPDIR:-/tmp}/sidenote-linux-tests.XXXXXX")
mkdir -p "$repo/target/test-storage"
for name in state meetings peers config data cache xdg-state public-test-key; do
    mkdir "$root/$name"
done
python3 - "$root" "$repo/target/test-storage" <<'PY'
import os
from pathlib import Path
import sys
for directory in map(Path, sys.argv[1:]):
    for path in (directory, *directory.parents):
        stat = path.stat()
        assert not stat.st_mode & 0o022 or stat.st_mode & 0o1000, f"unsafe test ancestor: {path}"
    assert directory.stat().st_uid == os.getuid(), f"test directory is not owned: {directory}"
key = Path(sys.argv[1]) / 'public-test-key/sidenote-key'
key.write_bytes(bytes(range(32)))  # Public synthetic fixture; never an owner's key.
key.chmod(0o600)
PY
export PATH="$HOME/.cargo/bin:$PATH"
export RUSTUP_TOOLCHAIN=stable CARGO_BUILD_JOBS=1
export SIDENOTE_DATA_DIR="$root/state" SIDENOTE_HEX_DIR="$root/meetings" SIDENOTE_PEERS_DIR="$root/peers"
export XDG_CONFIG_HOME="$root/config" XDG_DATA_HOME="$root/data" XDG_CACHE_HOME="$root/cache" XDG_STATE_HOME="$root/xdg-state"
export CREDENTIALS_DIRECTORY="$root/public-test-key"
export XDG_RUNTIME_DIR="/run/user/$(id -u)" DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$(id -u)/bus"
unset DISPLAY WAYLAND_DISPLAY
export CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUNNER
CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUNNER=$(python3 - "$repo/tools/linux_test_runner.py" "$root" <<'PY'
import shlex
import sys
print(shlex.join(['/usr/bin/python3', *sys.argv[1:]]))
PY
)
echo "Linux test fixture: $root (retained for inspection)" >&2
"$@"
