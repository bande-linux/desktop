#!/bin/bash
# hermetic.sh [--cargo] [--installed-services] <program> [arguments]: runs <program> as a test,
# with a fresh temporary HOME holding every XDG base directory, XDG_RUNTIME_DIR and GNUPGHOME,
# only the variables tests need (no display, Wayland socket, session or system bus, agent or
# locale of the person running it) and its own private session bus: the test bus
# (crates/testing/test-bus/data/session.conf), or with --installed-services the installed
# session bus configuration, which activates the installed services. The directory is removed
# afterwards. .cargo/config.toml makes it the runner of every test binary with --cargo, which
# runs any other program (cargo run, examples) as it is.
set -euo pipefail
tree=$(realpath "$(dirname "$0")/..")
bus_config=$tree/crates/testing/test-bus/data/session.conf
cargo=0
while [ $# -gt 0 ]; do
    case $1 in
        --cargo) cargo=1 ;;
        --installed-services) bus_config=/usr/share/dbus-1/session.conf ;;
        *) break ;;
    esac
    shift
done
[ $# -gt 0 ] || { echo "usage: hermetic.sh [--cargo] [--installed-services] <program> [arguments]" >&2; exit 2; }
if [ $cargo = 1 ] && [ "$(basename "$(dirname "$1")")" != deps ]; then
    exec "$@"
fi

kept=()
while IFS= read -r name; do
    case $name in
        PATH | USER | LOGNAME | TERM | TZ | LD_LIBRARY_PATH | SOURCE_DATE_EPOCH | \
            BANDE_EDID_CORPUS | CARGO* | RUST* | VK_* | MESA_* | LP_*)
            kept+=("$name=${!name}")
            ;;
    esac
done < <(compgen -e)

root=$(mktemp -d "${TMPDIR:-/tmp}/bande-test.XXXXXXXX")
trap 'chmod -R u+rwx "$root" 2> /dev/null || true; rm -rf "$root" || true' EXIT
mkdir -p "$root/home/.config" "$root/home/.local/share" "$root/home/.local/state" "$root/home/.cache" \
    "$root/tmp"
mkdir -m 700 "$root/runtime" "$root/home/.gnupg"

status=0
env -i "${kept[@]}" \
    BANDE_TEST_ROOT="$root" \
    HOME="$root/home" \
    XDG_CONFIG_HOME="$root/home/.config" \
    XDG_DATA_HOME="$root/home/.local/share" \
    XDG_STATE_HOME="$root/home/.local/state" \
    XDG_CACHE_HOME="$root/home/.cache" \
    XDG_RUNTIME_DIR="$root/runtime" \
    XDG_CONFIG_DIRS=/etc/xdg \
    XDG_DATA_DIRS=/usr/local/share:/usr/share \
    GNUPGHOME="$root/home/.gnupg" \
    TMPDIR="$root/tmp" \
    LANG=C.UTF-8 \
    DBUS_SYSTEM_BUS_ADDRESS="unix:path=$root/no-system-bus" \
    dbus-run-session --config-file="$bus_config" -- "$@" || status=$?
exit $status
