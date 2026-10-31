#!/bin/sh
set -eu
mkdir -p /run/dbus
dbus-daemon --system --fork
/usr/lib/polkit-1/polkitd --no-debug &
for attempt in $(seq 100); do
    busctl_reply=$(dbus-send --system --print-reply --dest=org.freedesktop.DBus / \
        org.freedesktop.DBus.NameHasOwner string:org.freedesktop.PolicyKit1 2>/dev/null || true)
    case "$busctl_reply" in *"boolean true"*) break ;; esac
    sleep 0.1
done
useradd --create-home tester
password=$(head -c 18 /dev/urandom | base64)
echo "tester:$password" | chpasswd
chmod 755 /polkitd-test
exec runuser -u tester -- env BANDE_POLKIT_PASSWORD="$password" /polkitd-test --ignored --test-threads=1 "$@"
