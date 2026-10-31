#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/../.."

debs=$(realpath "${1:-${CARGO_TARGET_DIR:-target}/debian}")
version=${DEB_VERSION:-$(ci/deb/version.sh)}
architecture=$(dpkg --print-architecture)
scratch=$(mktemp -d)
trap 'rm -rf "$scratch"' EXIT
for package in bande-config bande-compositor; do
    dpkg-deb --extract "$debs/${package}_${version}_$architecture.deb" "$scratch/root"
done

echo "== the packaged settings service answers on a private bus"
export XDG_CONFIG_HOME="$scratch/config"
mkdir -p "$XDG_CONFIG_HOME/bande"
echo 'bar { edge "top"; }' > "$XDG_CONFIG_HOME/bande/config.kdl"
dbus-run-session -- bash -c '
    "$1/usr/bin/bande-config" &
    daemon=$!
    for attempt in $(seq 50); do
        reply=$(dbus-send --session --print-reply --dest=org.bande.Config /org/bande/Config \
            org.freedesktop.DBus.Properties.Get string:org.bande.Config string:BarEdge 2>/dev/null) &&
            break
        sleep 0.1
    done
    kill "$daemon"
    echo "$reply"
    grep -q "string \"top\"" <<< "$reply"
' bash "$scratch/root"

echo "== the packaged compositor serves Wayland clients headless"
export XDG_RUNTIME_DIR="$scratch/runtime"
mkdir -m 700 "$XDG_RUNTIME_DIR"
status=0
timeout 10 "$scratch/root/usr/bin/bande-compositor" --headless > "$scratch/stdout" || status=$?
cat "$scratch/stdout"
[ "$status" = 124 ]
grep -q '^WAYLAND_DISPLAY=wayland-' "$scratch/stdout"
echo "the $architecture smoke tests passed"
