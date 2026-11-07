#!/bin/bash
# session.sh <out dir> <shot name> <seconds> <command...>: inside a test chroot, runs the command
# under the real headless Bande compositor on a private bus and a temporary runtime dir, with the
# portal running, and takes one screenshot after <seconds> into <out dir>/<shot name>.png. The
# compositor only lets the client it spawns itself capture, so grim stands in at /usr/bin/bande-review.
set -euo pipefail
out=$1 shot=$2 delay=$3; shift 3
run=$(mktemp -d)
chmod 700 "$run"
mkdir -p "$out"
cat > /usr/bin/bande-review <<REVIEW
#!/bin/sh
sleep $delay
grim "$out/$shot.png"
touch "$run/shot-done"
REVIEW
chmod +x /usr/bin/bande-review
export XDG_RUNTIME_DIR=$run XDG_SESSION_TYPE=wayland BANDE_REVIEW_JOB=1
export VK_DRIVER_FILES=$(ls /usr/share/vulkan/icd.d/lvp_icd.*.json | head -1)
export HOME=${HOME:-/root} XDG_CONFIG_HOME=${TEST_CONFIG:-$run/config} XDG_DATA_HOME=$run/data XDG_CACHE_HOME=$run/cache
exec dbus-run-session -- bash -c '
    run=$0 limit=$1; shift
    bande-compositor --headless > "$run/compositor.out" 2> "$run/compositor.err" &
    comp=$!
    for _ in $(seq 100); do grep -q "^WAYLAND_DISPLAY=" "$run/compositor.out" 2>/dev/null && break; sleep 0.2; done
    export $(grep "^WAYLAND_DISPLAY=" "$run/compositor.out")
    [ -n "${WAYLAND_DISPLAY:-}" ] || { cat "$run/compositor.err" >&2; exit 1; }
    [ -x /usr/bin/bande-config ] && { bande-config > "$run/config.log" 2>&1 & }
    /usr/libexec/xdg-desktop-portal-bande > "$run/portal.log" 2>&1 &
    "$@" &
    app=$!
    for _ in $(seq $((limit * 5))); do [ -e "$run/shot-done" ] && break; sleep 0.2; done
    sleep 1
    kill $app 2>/dev/null || true
    kill $comp 2>/dev/null || true
    wait 2>/dev/null || true
    [ -e "$run/shot-done" ]
' "$run" "$((delay + 60))" "$@"
