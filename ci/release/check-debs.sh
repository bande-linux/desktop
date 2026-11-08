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

if [ "${VERIFY_PROFILE:-dist}" = dist ]; then
    echo "== every program keeps only its function names, in .gnu_debugdata"
    for package in "${packages[@]}"; do
        root=$(extract "$package")
        programs=0
        while IFS= read -r file; do
            [ "$(od -An -tx1 -N4 "$file" | tr -d ' ')" = 7f454c46 ] || continue
            programs=$((programs + 1))
            sections=$(readelf --wide --section-headers "$file")
            if grep -q '\.debug_' <<< "$sections" || ! grep -q '\.gnu_debugdata' <<< "$sections"; then
                echo "${file#"$root"} in $package is not stripped to its .gnu_debugdata"
                exit 1
            fi
        done < <(find "$root" -type f)
        [ $programs -gt 0 ] || { echo "$package holds no program"; exit 1; }
    done
fi

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

echo "== the compositor ships the Bande pointers under their CSS and X11 names"
root=$(extract bande-compositor)
for name in default pointer text e-resize nwse-resize col-resize left_ptr right_side top_left_corner hand2 xterm; do
    head -c 4 "$root/usr/share/icons/Bande-Cursors/cursors/$name" | grep -q Xcur
done
grep -qx 'Name=Bande-Cursors' "$root/usr/share/icons/Bande-Cursors/index.theme"

echo "== each program carries its English catalog"
contains bande-files usr/share/bande/l10n/bande-files/en.mo
contains bande-shell usr/share/bande/l10n/bande-shell/en.mo

echo "== the notification daemon is D-Bus activatable"
root=$(extract bande-notify)
service="$root/usr/share/dbus-1/services/org.bande.Notify.service"
grep -qx 'Name=org.freedesktop.Notifications' "$service"
test -x "$root$(sed -n 's/^Exec=//p' "$service")"

echo "== the spelling service's package"
contains bande-spell usr/libexec/bande-spell
contains bande-spell usr/share/dbus-1/services/org.bande.Spell.service

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
grep -qx 'Exec=bande-files %U' "$root/usr/share/applications/org.bande.Files.desktop"
test -s "$root/usr/share/icons/hicolor/scalable/apps/org.bande.Files.svg"
root=$(extract bande-settings)
grep -qx 'Exec=bande-settings' "$root/usr/share/applications/org.bande.Settings.desktop"
root=$(extract bande-greeter)
grep -qx 'command = "/usr/libexec/bande-greeter-session"' "$root/usr/share/bande-greeter/greetd-config.toml"
field bande-greeter Depends | grep -qF 'greetd (>= 0.10.3-6)'
root=$(extract bande-lock)
grep -qx '@include common-auth' "$root/etc/pam.d/bande-lock"
field bande-lock Depends | grep -q libpam0g

echo "== the session services' user units and D-Bus activation"
for entry in bande-config:org.bande.Config:org.bande.Config \
    bande-power:org.bande.Power:org.freedesktop.ScreenSaver \
    bande-sound:org.bande.Sound:org.bande.Sound \
    bande-tray:org.bande.Tray:org.kde.StatusNotifierWatcher \
    bande-indexer:org.bande.Search:org.bande.Search; do
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

echo "== the keychain and its PAM module"
root=$(extract bande-secrets)
service="$root/usr/share/dbus-1/services/org.bande.Secrets.service"
grep -qx 'Name=org.freedesktop.secrets' "$service"
grep -qx 'SystemdService=bande-secrets.service' "$service"
test -x "$root$(sed -n 's/^Exec=//p' "$service")"
test -x "$root/usr/libexec/bande-secrets-prompt"
grep -qx 'ListenStream=%t/bande-secrets/pam' "$root/usr/lib/systemd/user/bande-secrets.socket"
grep -qx 'BusName=org.freedesktop.secrets' "$root/usr/lib/systemd/user/bande-secrets.service"
field bande-secrets Provides | grep -qw org.freedesktop.secrets
field bande-secrets Conflicts | grep -qw gnome-keyring
root=$(extract libpam-bande-secrets)
test -s "$root/usr/lib/$(dpkg-architecture -qDEB_HOST_MULTIARCH)/security/pam_bande_secrets.so"
grep -qP '^\toptional\tpam_bande_secrets.so$' "$root/usr/share/pam-configs/bande-secrets"
field libpam-bande-secrets Depends | grep -qw libpam0g
dpkg-deb --control "$debs/libpam-bande-secrets_${version}_$architecture.deb" "$root/DEBIAN"
grep -q 'pam-auth-update --package --remove bande-secrets' "$root/DEBIAN/prerm"

echo "== the automount service's user unit"
root=$(extract bande-automount)
unit="$root/usr/lib/systemd/user/bande-automount.service"
grep -qx 'WantedBy=bande-session.target' "$unit"
test -x "$root$(sed -n 's/^ExecStart=//p' "$unit")"
field bande-automount Depends | grep -q udisks2

echo "== the login session"
root="$scratch/session"
mkdir -p "$root"
for package in bande-compositor bande-config bande-power bande-tray bande-automount bande-secrets; do
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
grep -qx 'default=bande;' "$root/usr/share/xdg-desktop-portal/bande-portals.conf"
echo "the $architecture packages passed"
