#!/usr/bin/env bash
# Install only reviewed binaries; credential provisioning remains a separate admin step.
set -euo pipefail
umask 077
if test "$(id -u)" != 0 || test "$#" != 2; then
  echo 'Usage (root): install-linux-server.sh dedicated-uid build-directory' >&2
  exit 2
fi
account_uid=$1
build=$(realpath -- "$2")
case "$account_uid" in ''|*[!0-9]*|0) echo 'A dedicated non-root numeric uid is required' >&2; exit 2;; esac
getent passwd "$account_uid" >/dev/null
source_dir=$(dirname -- "$(realpath -- "$0")")
for binary in sidenote sidenote-linux-worker; do
  test -f "$build/$binary" && test ! -L "$build/$binary"
done
install -d -m 0755 /usr/local/libexec/sidenote
for binary in sidenote sidenote-linux-worker; do
  # Atomic replacement keeps the old running inode valid; stop before upgrading workers.
  install -m 0755 "$build/$binary" "/usr/local/libexec/sidenote/$binary.new"
  mv -f -- "/usr/local/libexec/sidenote/$binary.new" "/usr/local/libexec/sidenote/$binary"
done
install -m 0644 "$source_dir/systemd/sidenote-server@.service" /etc/systemd/system/sidenote-server@.service
install -m 0644 "$source_dir/systemd/sidenote-speech@.service" /etc/systemd/system/sidenote-speech@.service
systemctl daemon-reload
systemd-analyze verify /etc/systemd/system/sidenote-server@.service
systemd-analyze verify /etc/systemd/system/sidenote-speech@.service
echo "Installed. Provision the encrypted credential, then enable sidenote-server@$account_uid.service."
