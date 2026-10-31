#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/../.."

debs=$(realpath "${1:-${CARGO_TARGET_DIR:-target}/debian}")
version=${DEB_VERSION:-$(ci/deb/version.sh)}
architecture=$(dpkg --print-architecture)
mapfile -t packages < ci/release/packages
scratch=$(mktemp -d)
trap 'rm -rf "$scratch"' EXIT
extract() {
    local root="$scratch/$1"
    mkdir -p "$root"
    dpkg-deb --extract "$debs/$1_${version}_$architecture.deb" "$root"
    echo "$root"
}
field() { dpkg-deb --field "$debs/$1_${version}_$architecture.deb" "$2"; }
contains() { grep -q " ./$2\$" <<< "$(dpkg-deb --contents "$debs/$1_${version}_$architecture.deb")"; }

echo "== every package is built for $architecture at $version"
[ "$(find "$debs" -name '*.deb' | wc -l)" = "${#packages[@]}" ]
for package in "${packages[@]}"; do
    deb="$debs/${package}_${version}_$architecture.deb"
    dpkg-deb --info "$deb"
    dpkg-deb --contents "$deb"
    [ "$(dpkg-deb --field "$deb" Package)" = "$package" ]
    [ "$(dpkg-deb --field "$deb" Architecture)" = "$architecture" ]
    [ "$(dpkg-deb --field "$deb" Version)" = "$version" ]
    [ "$(dpkg-deb --field "$deb" Section)" = misc ]
    [ "$(dpkg-deb --fsys-tarfile "$deb" | tar -xO | grep -ac -e "$PWD" -e "${CARGO_HOME:-$HOME/.cargo}")" = 0 ]
done
field unbound-compositor Depends | grep -F 'mesa-vulkan-drivers | vulkan-icd'

echo "== the notification daemon is D-Bus activatable"
root=$(extract unbound-notify)
service="$root/usr/share/dbus-1/services/org.unbound.Notify.service"
grep -qx 'Name=org.freedesktop.Notifications' "$service"
test -x "$root$(sed -n 's/^Exec=//p' "$service")"

echo "== the clipboard daemon's package"
contains unbound-clipboard usr/bin/unbound-clipboard
field unbound-compositor Recommends | grep -qw unbound-clipboard

echo "== the shell's packages"
for package in unbound-shell unbound-term unbound-lock unbound-screenshot unbound-files unbound-settings; do
    field unbound-compositor Recommends | grep -q "$package"
    contains "$package" "usr/bin/$package"
done
for variant in light dark; do
    contains unbound-shell "usr/share/unbound/wallpapers/drift-$variant.svg"
done
root=$(extract unbound-term)
grep -qx 'Exec=unbound-term' "$root/usr/share/applications/org.unbound.Terminal.desktop"
grep -qx 'Icon=org.unbound.Terminal' "$root/usr/share/applications/org.unbound.Terminal.desktop"
test -s "$root/usr/share/icons/hicolor/scalable/apps/org.unbound.Terminal.svg"
root=$(extract unbound-files)
grep -qx 'Exec=unbound-files' "$root/usr/share/applications/org.unbound.Files.desktop"
test -s "$root/usr/share/icons/hicolor/scalable/apps/org.unbound.Files.svg"
root=$(extract unbound-settings)
grep -qx 'Exec=unbound-settings' "$root/usr/share/applications/org.unbound.Settings.desktop"
root=$(extract unbound-monitor)
test -x "$root/usr/bin/unbound-monitor"
grep -qx 'Exec=unbound-monitor' "$root/usr/share/applications/org.unbound.ActivityMonitor.desktop"
root=$(extract unbound-greeter)
grep -qx 'command = "/usr/bin/unbound-compositor --greeter"' "$root/usr/share/unbound-greeter/greetd-config.toml"
field unbound-greeter Depends | grep -qw greetd
root=$(extract unbound-lock)
grep -qx '@include common-auth' "$root/etc/pam.d/unbound-lock"
field unbound-lock Depends | grep -q libpam0g

echo "== the session services' user units and D-Bus activation"
for entry in unbound-config:org.unbound.Config:org.unbound.Config \
    unbound-power:org.unbound.Power:org.freedesktop.ScreenSaver \
    unbound-sound:org.unbound.Sound:org.unbound.Sound \
    unbound-tray:org.unbound.Tray:org.kde.StatusNotifierWatcher; do
    IFS=: read -r package file name <<< "$entry"
    root=$(extract "$package")
    service="$root/usr/share/dbus-1/services/$file.service"
    unit="$root/usr/lib/systemd/user/$package.service"
    grep -qx "Name=$name" "$service"
    grep -qx "SystemdService=$package.service" "$service"
    grep -qx "BusName=$name" "$unit"
    grep -qx 'WantedBy=unbound-session.target' "$unit"
    test -x "$root$(sed -n 's/^Exec=//p' "$service")"
    test -x "$root$(sed -n 's/^ExecStart=//p' "$unit")"
    dpkg-deb --ctrl-tarfile "$debs/${package}_${version}_$architecture.deb" | tar -t | grep -qx './postinst'
done

echo "== the automount service's user unit"
root=$(extract unbound-automount)
unit="$root/usr/lib/systemd/user/unbound-automount.service"
grep -qx 'WantedBy=unbound-session.target' "$unit"
test -x "$root$(sed -n 's/^ExecStart=//p' "$unit")"
field unbound-automount Depends | grep -q udisks2

echo "== the login session"
root="$scratch/session"
mkdir -p "$root"
for package in unbound-compositor unbound-config unbound-power unbound-tray unbound-automount; do
    dpkg-deb --extract "$debs/${package}_${version}_$architecture.deb" "$root"
done
session="$root/usr/share/wayland-sessions/unbound.desktop"
grep -qx 'Type=Application' "$session"
grep -qx 'Name=Unbound' "$session"
grep -qx 'Exec=unbound-session' "$session"
grep -qx 'DesktopNames=Unbound' "$session"
test -x "$root/usr/bin/unbound-session"
sh -n "$root/usr/bin/unbound-session"
units="$scratch/units"
mkdir -p "$units"
cp "$root"/usr/lib/systemd/user/* "$units/"
sed -i "s|^ExecStart=/|ExecStart=$root/|" "$units"/*
grep -qx 'Type=notify' "$units/unbound-compositor.service"
grep -qx 'BindsTo=graphical-session.target' "$units/unbound-compositor.service"
grep -qx 'Wants=xdg-desktop-autostart.target' "$units/unbound-session.target"
[ -z "$(grep -l 'WantedBy=graphical-session.target' "$units"/* || true)" ]
findings=$(cd "$units" && XDG_RUNTIME_DIR=$(mktemp -d -p "$scratch") SYSTEMD_LOG_LEVEL=warning systemd-analyze --user verify ./* 2>&1) || true
echo "$findings"
test -z "$findings"

echo "== the portal backend's registration"
root=$(extract xdg-desktop-portal-unbound)
portal="$root/usr/share/xdg-desktop-portal/portals/unbound.portal"
service="$root/usr/share/dbus-1/services/org.freedesktop.impl.portal.desktop.unbound.service"
grep -qx 'DBusName=org.freedesktop.impl.portal.desktop.unbound' "$portal"
grep -qx 'Name=org.freedesktop.impl.portal.desktop.unbound' "$service"
test -x "$root$(sed -n 's/^Exec=//p' "$service")"
grep -q '^org.freedesktop.impl.portal.Settings=unbound;' "$root/usr/share/xdg-desktop-portal/unbound-portals.conf"
echo "the $architecture packages passed"
