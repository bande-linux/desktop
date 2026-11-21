#!/bin/bash
# session.sh <command...>: inside a test chroot, runs the command under the real headless Bande
# compositor on a private bus and runtime dir, with the portal running, and returns its exit code.
# The compositor only lets the client it spawns capture the screen, so scripts/shot.py stands in at
# /usr/bin/bande-review; `SHOT_DIR` tells the command where to ask for screenshots.
set -euo pipefail
repo=$(dirname "$(dirname "$(realpath "$0")")")
run=$(mktemp -d)
chmod 700 "$run"
mkdir -p "$run/shots"
export SHOT_DIR=$run/shots
printf '#!/bin/sh\nexec python3 -I %s/scripts/shot.py %s\n' "$repo" "$SHOT_DIR" > /usr/bin/bande-review
chmod +x /usr/bin/bande-review
export XDG_RUNTIME_DIR=$run XDG_SESSION_TYPE=wayland BANDE_REVIEW_JOB=1
export VK_DRIVER_FILES=/usr/share/vulkan/icd.d/lvp_icd.json
export HOME=${HOME:-/root} XDG_CONFIG_HOME=${TEST_CONFIG:-$run/config} XDG_DATA_HOME=$run/data XDG_CACHE_HOME=$run/cache
mkdir -p "$XDG_CONFIG_HOME"
exec dbus-run-session -- bash -c '
    run=$0
    mkfifo "$run/compositor.pipe"
    bande-compositor --headless > "$run/compositor.pipe" 2> "$run/compositor.err" &
    compositor=$!
    exec {out}< "$run/compositor.pipe"
    rm "$run/compositor.pipe"
    display=
    while IFS= read -r -t 20 line <&"$out"; do
        printf "%s\n" "$line" >> "$run/compositor.out"
        [[ $line != WAYLAND_DISPLAY=* ]] || { display=$line; break; }
    done
    [ -n "$display" ] || { cat "$run/compositor.err" >&2; exit 1; }
    cat <&"$out" >> "$run/compositor.out" &
    exec {out}<&-
    export "$display"
    bande-config > "$run/config.log" 2>&1 &
    config=$!
    /usr/libexec/xdg-desktop-portal-bande > "$run/portal.log" 2>&1 &
    portal=$!
    set +e
    "$@"
    status=$?
    kill "$compositor" "$config" "$portal" 2>/dev/null
    wait
    exit $status
' "$run" "$@"
