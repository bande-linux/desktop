#!/bin/sh
set -eu
export XDG_CURRENT_DESKTOP=Unbound
export XDG_CONFIG_HOME="$(mktemp -d)"
mkdir -p "$XDG_CONFIG_HOME/unbound"
echo 'appearance "dark"' > "$XDG_CONFIG_HOME/unbound/config.kdl"
exec dbus-run-session -- sh -eu -c '
    color_scheme() {
        dbus-send --session --print-reply --dest=org.freedesktop.portal.Desktop \
            /org/freedesktop/portal/desktop org.freedesktop.portal.Settings.ReadOne \
            string:org.freedesktop.appearance string:color-scheme
    }
    wait_for() {
        for attempt in $(seq 100); do
            reply=$(color_scheme 2>&1 || true)
            case "$reply" in *"uint32 $1"*) echo "color-scheme is $1"; return 0 ;; esac
            sleep 0.1
        done
        echo "expected color-scheme $1, got: $reply"
        return 1
    }
    wait_for 1
    dbus-send --session --print-reply --dest=org.unbound.Config /org/unbound/Config \
        org.freedesktop.DBus.Properties.Set string:org.unbound.Config string:Appearance \
        variant:string:light > /dev/null
    wait_for 2
    dbus-send --session --print-reply --dest=org.freedesktop.portal.Desktop \
        /org/freedesktop/portal/desktop org.freedesktop.portal.Settings.ReadOne \
        string:org.freedesktop.appearance string:accent-color | grep -c double | grep -qx 3
    echo "accent-color is served"
'
