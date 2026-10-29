#!/bin/bash
set -euo pipefail

images=$(realpath "${1:?usage: guest-tests.sh <images-dir>}")
here=$(dirname "$(realpath "$0")")
share=$(mktemp -d)
trap 'rm -rf "$share"' EXIT

cargo_json() {
    if ! cargo "$@" --message-format=json > "$share/cargo.json"; then
        jq -r 'select(.reason == "compiler-message") | .message.rendered' "$share/cargo.json" >&2
        return 1
    fi
}

cargo_json test --workspace --locked --no-run
jq -r 'select(.reason == "compiler-artifact" and .target.name == "guest" and .executable != null) | .executable' \
    "$share/cargo.json" > "$share/executables"
if [ ! -s "$share/executables" ]; then
    echo "No guest test executables were built" >&2
    exit 1
fi

cargo_json build --workspace --locked --bins
jq -r 'select(.reason == "compiler-artifact" and .executable != null and .target.kind == ["bin"]) | .executable' \
    "$share/cargo.json" > "$share/apps"
mkdir -p "$share/usr/bin" "$share/usr/share/applications" "$share/usr/share/icons/hicolor/scalable/apps"
for app in unbound-compositor unbound-clipboard unbound-dock unbound-launcher unbound-term unbound-lock unbound-screenshot unbound-files unbound-greeter unbound-menubar; do
    cp "$(grep "/$app\$" "$share/apps")" "$share/usr/bin/"
done
mkdir -p "$share/etc/pam.d"
cp crates/shell/lock/data/unbound-lock.pam "$share/etc/pam.d/unbound-lock"
cp crates/apps/terminal/data/unbound-term.desktop "$share/usr/share/applications/"
cp crates/apps/terminal/data/unbound-term.svg "$share/usr/share/icons/hicolor/scalable/apps/"

mkdir "$share/bin"
while read -r executable; do
    cp "$executable" "$share/bin/$(basename "$executable")"
done < "$share/executables"

cat > "$share/run" <<'GUEST'
set -u
export RUST_BACKTRACE=1 UNBOUND_SCREENDUMPS=/share/screendumps
echo "Guest kernel $(uname -r)"
ls -l /dev/dri
cp -rs /share/usr /share/etc /
/usr/lib/systemd/systemd-udevd --daemon
udevadm trigger --action=add
udevadm settle
SEATD_VTBOUND=0 seatd -l error &
for _ in $(seq 50); do
    [ -S /run/seatd.sock ] && break
    sleep 0.1
done
status=0
for test in bin/*; do
    echo "Running ${test#bin/} in the guest"
    "$test" --ignored --test-threads=1 || status=1
done
exit "$status"
GUEST

status=0
"$here/run.sh" "$images" "$share" || status=$?
if [ -d "$share/screendumps" ]; then
    mkdir -p screendumps
    cp "$share"/screendumps/*.png screendumps/
fi
exit "$status"
