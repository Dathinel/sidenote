#!/usr/bin/env bash
set -euo pipefail
umask 077
test "$(uname -s)" = Linux
test "$#" -eq 1 || { echo 'Pass the directory containing the three Linux binaries.' >&2; exit 2; }
owner=$(id -un)
uid=$(id -u "$owner")
test "$uid" -ne 0 || { echo 'Run this installer as the unprivileged service owner.' >&2; exit 2; }
root=$(git rev-parse --show-toplevel)
binaries=$1
sudo test -f "/etc/credstore.encrypted/sidenote-$uid"
install -d -m 700 "$HOME/.local/bin" "$HOME/.config/systemd/user/sidenote.service.d"
for binary in sidenote sidenote-sync-worker sidenote-linux-worker; do
    install -m 755 "$binaries/$binary" "$HOME/.local/bin/$binary"
done
sudo install -m 644 "$root/packaging/systemd/sidenote-credentials@.service" /etc/systemd/system/
dropin=$(mktemp)
trap 'rm -f "$dropin"' EXIT
printf '[Unit]\nAfter=sidenote-credentials@%s.service\nWants=sidenote-credentials@%s.service\n' "$uid" "$uid" > "$dropin"
sudo install -d -m 755 "/etc/systemd/system/user@$uid.service.d"
sudo install -m 644 "$dropin" "/etc/systemd/system/user@$uid.service.d/sidenote-credentials.conf"
sudo systemctl daemon-reload
sudo systemctl enable --now "sidenote-credentials@$uid.service"
install -m 644 "$root/packaging/systemd/sidenote.service" "$HOME/.config/systemd/user/sidenote.service"
printf '[Service]\nLoadCredential=sidenote-key:/run/credentials/sidenote-credentials@%s.service/sidenote-key\n' "$uid" > "$HOME/.config/systemd/user/sidenote.service.d/credential.conf"
sudo loginctl enable-linger "$owner"
systemctl --user daemon-reload
systemctl --user enable --now sidenote.service
loginctl show-user "$owner" -p Linger
