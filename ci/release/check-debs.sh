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
    section=$(dpkg-deb --field "$deb" Section)
    [[ " misc admin " == *" $section "* ]] || { echo "$package is in section $section, not misc or admin"; exit 1; }
    [ "$(dpkg-deb --fsys-tarfile "$deb" | tar -xO | grep -ac -e "$PWD" -e "${CARGO_HOME:-$HOME/.cargo}")" = 0 ]
done
field bande-compositor Depends | grep -F 'mesa-vulkan-drivers | vulkan-icd'

echo "== the notification daemon is D-Bus activatable"
root=$(extract bande-notify)
service="$root/usr/share/dbus-1/services/org.bande.Notify.service"
grep -qx 'Name=org.freedesktop.Notifications' "$service"
test -x "$root$(sed -n 's/^Exec=//p' "$service")"

echo "== the clipboard daemon's package"
contains bande-clipboard usr/bin/bande-clipboard
field bande-compositor Recommends | grep -qw bande-clipboard

echo "== the shell's packages"
for package in bande-shell bande-lock bande-screenshot bande-files bande-settings; do
    field bande-compositor Recommends | grep -q "$package"
    contains "$package" "usr/bin/$package"
done
for variant in light dark; do
    contains bande-shell "usr/share/bande/wallpapers/drift-$variant.svg"
done
root=$(extract bande-files)
grep -qx 'Exec=bande-files' "$root/usr/share/applications/org.bande.Files.desktop"
test -s "$root/usr/share/icons/hicolor/scalable/apps/org.bande.Files.svg"
root=$(extract bande-settings)
grep -qx 'Exec=bande-settings' "$root/usr/share/applications/org.bande.Settings.desktop"
root=$(extract bande-greeter)
grep -qx 'command = "/usr/bin/bande-compositor --greeter"' "$root/usr/share/bande-greeter/greetd-config.toml"
field bande-greeter Depends | grep -qF 'greetd (>= 0.10.3-6)'
root=$(extract bande-lock)
grep -qx '@include common-auth' "$root/etc/pam.d/bande-lock"
field bande-lock Depends | grep -q libpam0g

echo "== the session services' user units and D-Bus activation"
for entry in bande-config:org.bande.Config:org.bande.Config \
    bande-power:org.bande.Power:org.freedesktop.ScreenSaver \
    bande-sound:org.bande.Sound:org.bande.Sound \
    bande-tray:org.bande.Tray:org.kde.StatusNotifierWatcher; do
    IFS=: read -r package file name <<< "$entry"
    root=$(extract "$package")
    service="$root/usr/share/dbus-1/services/$file.service"
    unit="$root/usr/lib/systemd/user/$package.service"
    grep -qx "Name=$name" "$service"
    grep -qx "SystemdService=$package.service" "$service"
    grep -qx "BusName=$name" "$unit"
    grep -qx 'WantedBy=bande-session.target' "$unit"
    test -x "$root$(sed -n 's/^Exec=//p' "$service")"
    test -x "$root$(sed -n 's/^ExecStart=//p' "$unit")"
    dpkg-deb --control "$debs/${package}_${version}_$architecture.deb" "$root/DEBIAN"
    test -x "$root/DEBIAN/postinst"
done

echo "== the automount service's user unit"
root=$(extract bande-automount)
unit="$root/usr/lib/systemd/user/bande-automount.service"
grep -qx 'WantedBy=bande-session.target' "$unit"
test -x "$root$(sed -n 's/^ExecStart=//p' "$unit")"
field bande-automount Depends | grep -q udisks2

echo "== the login session"
root="$scratch/session"
mkdir -p "$root"
for package in bande-compositor bande-config bande-power bande-tray bande-automount; do
    dpkg-deb --extract "$debs/${package}_${version}_$architecture.deb" "$root"
done
session="$root/usr/share/wayland-sessions/bande.desktop"
grep -qx 'Type=Application' "$session"
grep -qx 'Name=Bande' "$session"
grep -qx 'Exec=bande-session' "$session"
grep -qx 'DesktopNames=Bande' "$session"
test -x "$root/usr/bin/bande-session"
sh -n "$root/usr/bin/bande-session"
units="$scratch/units"
mkdir -p "$units"
cp "$root"/usr/lib/systemd/user/* "$units/"
sed -i "s|^ExecStart=/|ExecStart=$root/|" "$units"/*
grep -qx 'Type=notify' "$units/bande-compositor.service"
grep -qx 'BindsTo=graphical-session.target' "$units/bande-compositor.service"
grep -qx 'Wants=xdg-desktop-autostart.target' "$units/bande-session.target"
[ -z "$(grep -l 'WantedBy=graphical-session.target' "$units"/* || true)" ]
findings=$(cd "$units" && XDG_RUNTIME_DIR=$(mktemp -d -p "$scratch") SYSTEMD_LOG_LEVEL=warning systemd-analyze --user verify ./* 2>&1) || true
echo "$findings"
test -z "$findings"

echo "== the portal backend's registration"
root=$(extract xdg-desktop-portal-bande)
portal="$root/usr/share/xdg-desktop-portal/portals/bande.portal"
service="$root/usr/share/dbus-1/services/org.freedesktop.impl.portal.desktop.bande.service"
grep -qx 'DBusName=org.freedesktop.impl.portal.desktop.bande' "$portal"
grep -qx 'Name=org.freedesktop.impl.portal.desktop.bande' "$service"
test -x "$root$(sed -n 's/^Exec=//p' "$service")"
grep -q '^org.freedesktop.impl.portal.Settings=bande;' "$root/usr/share/xdg-desktop-portal/bande-portals.conf"
echo "the $architecture packages passed"
