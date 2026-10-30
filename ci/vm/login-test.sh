#!/bin/bash
set -euo pipefail

images=$(realpath "${1:?usage: login-test.sh <images-dir>}")
here=$(dirname "$(realpath "$0")")
share=$(mktemp -d)
trap 'rm -rf "$share"' EXIT

cargo_json() {
    if ! cargo "$@" --message-format=json > "$share/cargo.json"; then
        jq -r 'select(.reason == "compiler-message") | .message.rendered' "$share/cargo.json" >&2
        return 1
    fi
}

cargo_json test --locked -p unbound-greeter --test login --no-run
jq -r 'select(.reason == "compiler-artifact" and .target.name == "login" and .executable != null) | .executable' \
    "$share/cargo.json" > "$share/executable"
cargo_json build --locked -p unbound-compositor -p unbound-greeter -p unbound-shell --bins
jq -r 'select(.reason == "compiler-artifact" and .executable != null and .target.kind == ["bin"]) | .executable' \
    "$share/cargo.json" > "$share/apps"

root=$share/root
mkdir -p "$root/usr/bin" "$root/usr/lib/systemd/user" "$root/usr/lib/systemd/system/greetd.service.d" \
    "$root/usr/share/wayland-sessions" "$root/usr/share/unbound-greeter" "$root/usr/share/unbound/wallpapers"
for app in unbound-compositor unbound-greeter unbound-shell; do
    cp "$(grep "/$app\$" "$share/apps")" "$root/usr/bin/"
done
compositor=crates/platform/compositor/data
greeter=crates/shell/greeter
cp "$compositor/unbound-session" "$root/usr/bin/"
cp data/wallpapers/*.svg "$root/usr/share/unbound/wallpapers/"
cp "$compositor/unbound-compositor.service" "$compositor/unbound-session.target" "$root/usr/lib/systemd/user/"
cp "$compositor/unbound.desktop" "$root/usr/share/wayland-sessions/"
cp "$greeter/data/config.toml" "$root/usr/share/unbound-greeter/greetd-config.toml"
cp "$greeter/data/greetd.service.conf" "$root/usr/lib/systemd/system/greetd.service.d/unbound.conf"
cp "$greeter/debian/postinst" "$share/greeter-postinst"
cp "$(cat "$share/executable")" "$share/login"

cat > "$share/run" <<'GUEST'
set -u
export RUST_BACKTRACE=1
cp -r /share/root/. /
chmod 755 /usr/bin/unbound-session
printf 'XKBMODEL="pc105"\nXKBLAYOUT="se"\nXKBVARIANT=""\nXKBOPTIONS=""\n' > /etc/default/keyboard
useradd --create-home --shell /bin/bash --comment "Ada Lovelace" ada
echo 'ada:kåäö1' | chpasswd
sh /share/greeter-postinst configure ""
systemctl daemon-reload
systemctl start display-manager.service
status=0
/share/login --ignored --test-threads=1 --nocapture || status=$?
if [ "$status" -ne 0 ]; then
    systemctl --no-pager status greetd.service
    journalctl --no-pager -b -o short-monotonic | tail -n 200
fi
echo "$status" > /share/status
sync
systemctl poweroff --force --force
GUEST

GUEST_INIT=/usr/lib/systemd/systemd GUEST_APPEND="systemd.wants=unbound-guest-login.service" \
    "$here/run.sh" "$images" "$share"
