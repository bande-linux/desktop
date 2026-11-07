#!/bin/bash
set -euo pipefail

old=$1
new=$2
user=tester
export DEBIAN_FRONTEND=noninteractive
failures=0

say() { echo "== $*"; }
pass() { echo "ok: $*"; }
fail() { echo "FAILED: $*"; failures=$((failures + 1)); }
check() {
    local what=$1
    shift
    if "$@"; then pass "$what"; else fail "$what"; fi
}
as_user() { runuser -u "$user" -- env XDG_RUNTIME_DIR="/run/user/$uid" "$@"; }

wait_until() {
    local what=$1 patience=$2 end
    shift 2
    end=$((SECONDS + patience))
    until "$@"; do
        if [ "$SECONDS" -ge "$end" ]; then
            fail "$what"
            return 0
        fi
        sleep 0.5
    done
    pass "$what"
}

pid_of() { pgrep -u "$user" -x "$1" | head -n1; }
running() { [ -n "$(pid_of "$1")" ]; }
restarted() { local now; now=$(pid_of "$1"); [ -n "$now" ] && [ "$now" != "$2" ]; }
running_sum() { sha256sum "/proc/$1/exe" | cut -d' ' -f1; }

packaged_sum() {
    local package=$1 binary=$2 directory=$3 extract
    extract=$(mktemp -d)
    dpkg-deb --extract "$directory/${package}"_*.deb "$extract"
    sha256sum "$extract/usr/bin/$binary" | cut -d' ' -f1
    rm -rf "$extract"
}

is_build() {
    local binary=$1 wanted=$2 pid
    pid=$(pid_of "$binary")
    [ -n "$pid" ] && [ "$(running_sum "$pid")" = "$wanted" ]
}

say "install the old version"
apt-get update -qq
apt-get install -y -qq --no-install-recommends \
    mesa-vulkan-drivers pipewire-bin dbus-user-session polkitd > /dev/null
apt-get install -y -qq --no-install-recommends "$old"/*.deb > /dev/null

say "log a user in on tty1 with a headless compositor"
useradd -m -s /bin/bash "$user"
uid=$(id -u "$user")
home=$(getent passwd "$user" | cut -d: -f6)
override=$home/.config/systemd/user/bande-compositor.service.d
mkdir -p "$override"
cat > "$override/headless.conf" <<UNIT
[Service]
ExecStart=
ExecStart=/usr/bin/bande-compositor --headless
Environment=VK_DRIVER_FILES=/usr/share/vulkan/icd.d/lvp_icd.json
UNIT
cat >> "$home/.profile" <<'LOGIN'
[ "$(tty)" = /dev/tty1 ] && exec bande-session
LOGIN
chown -R "$user:" "$home"
mkdir -p /etc/systemd/system/getty@tty1.service.d
cat > /etc/systemd/system/getty@tty1.service.d/login.conf <<UNIT
[Service]
ExecStart=
ExecStart=-/sbin/agetty --autologin $user --noclear %I \$TERM
UNIT
systemctl daemon-reload
systemctl restart getty@tty1.service

wait_until "the session's shell runs" 90 running bande-shell
wait_until "the session's clipboard runs" 30 running bande-clipboard
wait_until "the settings service runs" 30 running bande-config
check "the update watcher is active" as_user systemctl --user is-active --quiet bande-update-finish.path

old_shell=$(packaged_sum bande-shell bande-shell "$old")
old_lock=$(packaged_sum bande-lock bande-lock "$old")
new_config=$(packaged_sum bande-config bande-config "$new")
new_shell=$(packaged_sum bande-shell bande-shell "$new")
[ "$old_shell" != "$new_shell" ] || { echo "FAILED: the old and new shell are the same build"; exit 1; }
check "the shell is the old build before the update" is_build bande-shell "$old_shell"
compositor=$(pid_of bande-compositor)

monitor=$(mktemp)
as_user env DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$uid/bus" \
    dbus-monitor --session "interface='org.freedesktop.Notifications',member='Notify'" > "$monitor" 2>&1 &
watcher=$!

say "update the running session with apt"
apt-get install -y -qq --no-install-recommends "$new"/*.deb > /dev/null
check "the new shell is installed" [ "$(sha256sum /usr/bin/bande-shell | cut -d' ' -f1)" = "$new_shell" ]

wait_until "the settings service moves to the new build" 30 is_build bande-config "$new_config"
told() { grep -q "Log out to finish updating" "$monitor"; }
wait_until "the user is told to log out to finish" 30 told
check "the update did not end the session" [ "$(pid_of bande-compositor)" = "$compositor" ]

say "the shell crashes after the update"
shell=$(pid_of bande-shell)
kill -9 "$shell"
wait_until "the shell comes back" 30 restarted bande-shell "$shell"
wait_until "the shell that came back is the session's own build" 5 is_build bande-shell "$old_shell"
shell=$(pid_of bande-shell)
sleep 15
check "the shell stays up" [ "$(pid_of bande-shell)" = "$shell" ]

say "the screen locks after the update"
loginctl lock-sessions
wait_until "the lock screen starts" 30 running bande-lock
wait_until "the lock screen is the session's own build" 5 is_build bande-lock "$old_lock"
lock=$(pid_of bande-lock)
sleep 12
check "the lock screen stays up" [ "$(pid_of bande-lock)" = "$lock" ]
kill -9 "$lock"
wait_until "a killed lock screen comes back" 30 restarted bande-lock "$lock"
wait_until "the lock screen that came back is the session's own build" 5 is_build bande-lock "$old_lock"
check "the held lock screen keeps its process name" [ "$(cat "/proc/$(pid_of bande-lock)/comm")" = bande-lock ]

say "PipeWire restarts"
has_shell_client() { as_user pw-cli list-objects Client 2>/dev/null | grep -q 'application.process.binary = "bande-shell"'; }
wait_until "the shell is connected to PipeWire" 30 has_shell_client
as_user systemctl --user restart pipewire.service
sleep 1
wait_until "the shell reconnects to PipeWire" 30 has_shell_client

say "a new app runs on the old compositor"
as_user env DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$uid/bus" \
    WAYLAND_DISPLAY="$(as_user systemctl --user show-environment | sed -n 's/^WAYLAND_DISPLAY=//p')" \
    systemd-run --user --quiet --unit=upgrade-test-settings /usr/bin/bande-settings
sleep 5
check "new Settings stays open" as_user systemctl --user is-active --quiet upgrade-test-settings.service

kill "$watcher" 2>/dev/null || true
if journalctl --user-unit bande-compositor.service --no-pager -q | grep -E 'message too short|Wayland dispatch failed'; then
    fail "the journal shows a protocol mismatch"
fi

if [ "$failures" -ne 0 ]; then
    journalctl -b --no-pager -q -n 200 | cut -c1-200
    echo "$failures checks failed"
    exit 1
fi
echo "the session survived the update"
