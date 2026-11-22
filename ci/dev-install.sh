#!/bin/bash
# dev-install.sh <program>...: builds the named programs with the release profile in the
# workspace's own target/ and installs each over the copy its package installed, at the path
# debian/*.install gives it, then restarts the user services that run it. The packages must be
# installed first; a reinstall of the package puts its own copy back.
set -euo pipefail
cd "$(dirname "$0")/.."
[ $# -gt 0 ] || { echo "usage: dev-install.sh <program>..." >&2; exit 2; }

destinations=()
for program; do
    line=$(grep -h "^debian/build/bin/$program " debian/*.install || true)
    [ -n "$line" ] || { echo "dev-install.sh: no package installs $program" >&2; exit 1; }
    destination=/${line#* }
    destination=${destination%/}/$program
    [ -e "$destination" ] || { echo "dev-install.sh: $destination is not installed; install its package first" >&2; exit 1; }
    destinations+=("$destination")
done

bins=()
for program; do bins+=(--bin "$program"); done
cargo build --release --locked "${bins[@]}"

for i in "${!destinations[@]}"; do
    sudo install -m 0755 "target/release/${*:i+1:1}" "${destinations[$i]}"
    echo "installed ${destinations[$i]}"
done

for destination in "${destinations[@]}"; do
    grep -l "^ExecStart=-\?$destination\( \|$\)" /usr/lib/systemd/user/*.service 2>/dev/null |
    while read -r unit; do
        name=$(basename "$unit")
        if systemctl --user is-active --quiet "$name"; then
            systemctl --user restart "$name"
            echo "restarted $name"
        fi
    done
    if grep -lq "^ExecStart=-\?$destination\( \|$\)" /usr/lib/systemd/system/*.service 2>/dev/null; then
        echo "$destination runs as a system service: sudo systemctl restart it"
    fi
    case ${destination##*/} in
        bande-compositor | bande-shell | bande-greeter | bande-lock)
            echo "${destination##*/} is part of the session: log out and in to use the new one" ;;
    esac
done
