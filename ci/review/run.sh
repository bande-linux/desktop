#!/bin/bash
set -euo pipefail

repo=$(cd "$(dirname "$0")/../.." && pwd)
cargo=${REVIEW_CARGO:-cargo}
packages=(
    unbound-compositor unbound-notify unbound-clipboard unbound-config unbound-power
    xdg-desktop-portal-unbound unbound-tray unbound-automount
    unbound-dock unbound-launcher unbound-term unbound-lock unbound-screenshot unbound-files
    unbound-settings unbound-greeter unbound-menubar unbound-controlcenter
)
screenshot=/usr/bin/unbound-screenshot
runtime="${XDG_RUNTIME_DIR:?needs a user session}/unbound-review"
build=1
take_over=0
fps=30
scenarios=()

while [ $# -gt 0 ]; do
    case $1 in
        --no-build) build=0 ;;
        --take-over) take_over=1 ;;
        --fps) fps=${2:?--fps needs a number}; shift ;;
        -*) echo "run.sh: unknown option $1" >&2; exit 2 ;;
        *)
            if [ -f "$1" ]; then
                scenarios+=("$(realpath "$1")")
            elif [ -f "$repo/ci/review/scenarios/$1.scenario" ]; then
                scenarios+=("$repo/ci/review/scenarios/$1.scenario")
            else
                echo "run.sh: no scenario $1" >&2
                exit 2
            fi
            ;;
    esac
    shift
done
if [ ${#scenarios[@]} -eq 0 ]; then
    echo "usage: ci/review/run.sh [--no-build] [--take-over] [--fps N] <scenario>..." >&2
    echo "Builds and installs this checkout's packages, then records each scenario on a fresh" >&2
    echo "session into review-out/<scenario>-<time>/index.html. REVIEW_CARGO picks the cargo command." >&2
    echo "scenarios: $(cd "$repo/ci/review/scenarios" && ls -- *.scenario | sed 's/\.scenario$//' | xargs)" >&2
    exit 2
fi

command -v ffmpeg >/dev/null || "$repo/ci/apt-install.sh" ffmpeg

cd "$repo"
if [ "$build" = 1 ]; then
    version=$(ci/deb/version.sh "$repo")
    echo "Building packages $version"
    debs=()
    for package in "${packages[@]}"; do
        $cargo deb --package "$package" --locked --deb-version "$version"
        debs+=("$repo/target/debian/${package}_${version}_arm64.deb")
    done
    sudo apt-get install -y --allow-downgrades --reinstall "${debs[@]}"
fi
$cargo build --release --locked --package unbound-review
review="$repo/target/release/unbound-review"

stop_session() {
    if systemctl --user --quiet is-active unbound-compositor.service; then
        echo "Stopping the running session"
        systemctl --user stop unbound-compositor.service
    fi
    if pgrep -u "$(id -u)" -x unbound-composi >/dev/null; then
        if [ "$take_over" != 1 ]; then
            echo "run.sh: an Unbound compositor runs outside the session service (the console session);" >&2
            echo "run.sh: pass --take-over to stop it for the recording" >&2
            exit 1
        fi
        echo "Stopping the Unbound compositor started outside the session service"
        pkill -TERM -u "$(id -u)" -x unbound-composi
    fi
}

cleanup() {
    if findmnt -n "$screenshot" >/dev/null 2>&1; then
        sudo umount --lazy "$screenshot" || true
    fi
    sudo chown root:root /dev/uinput || true
    rm -f "$runtime/job"
}
trap cleanup EXIT

wait_for() {
    local seconds=$1 what=$2
    shift 2
    for _ in $(seq $((seconds * 10))); do
        "$@" && return 0
        sleep 0.1
    done
    echo "run.sh: timed out waiting for $what" >&2
    return 1
}

record() {
    local scenario=$1
    local name out cursor
    name=$(basename "$scenario" .scenario)
    out="$repo/review-out/$name-$(date +%Y%m%d-%H%M%S)"
    mkdir -p "$out" "$runtime"
    echo "Recording $name into $out"

    stop_session
    wait_for 30 "the old session to stop" \
        bash -c '! systemctl --user --quiet is-active unbound-compositor.service && ! pgrep -u "$(id -u)" -x unbound-composi >/dev/null'
    cp "$screenshot" "$runtime/unbound-screenshot"
    {
        printf 'REVIEW_BIN=%q\n' "$review"
        printf 'SCENARIO=%q\n' "$scenario"
        printf 'OUT=%q\n' "$out"
        printf 'FPS=%q\n' "$fps"
    } > "$runtime/job"
    sudo chown "$(id -u)" /dev/uinput
    sudo mount --bind "$repo/ci/review/shell-client" "$screenshot"

    cursor=$(journalctl --user -n 0 --show-cursor --no-pager | sed -n 's/^-- cursor: //p')
    systemctl --user set-environment LIBSEAT_BACKEND=seatd
    setsid -f unbound-session > "$out/session.log" 2>&1 < /dev/null
    wait_for 60 "the session to start unbound-review" bash -c "[ ! -e '$runtime/job' ]"
    sudo umount --lazy "$screenshot"

    wait_for 1800 "the scenario to finish" test -e "$out/exit-status"
    sudo chown root:root /dev/uinput
    journalctl --user --after-cursor "$cursor" --no-pager -o cat -t unbound-review > "$out/review.log" || true
    journalctl --user --after-cursor "$cursor" --no-pager -o short-monotonic > "$out/journal.log" || true
    if [ "$(cat "$out/exit-status")" != 0 ]; then
        echo "run.sh: $name did not finish cleanly; see $out/review.log" >&2
        tail -n 5 "$out/review.log" >&2 || true
        failed=1
    fi
    echo "Review: $out/index.html"
}

exec 8> /tmp/unbound-live-session.lock
if ! flock -n 8; then
    echo "Waiting for the live session lock (/tmp/unbound-live-session.lock)"
    flock 8
fi

failed=0
for scenario in "${scenarios[@]}"; do
    record "$scenario"
done
exit "$failed"
