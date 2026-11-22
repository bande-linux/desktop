#!/bin/bash
# check-debs.sh <debs dir> [version]: checks the binary packages one build of debian/ made for this
# architecture and the architecture-independent ones: versions, stripping, contents and relations.
set -euo pipefail
cd "$(dirname "$0")/../.."

debs=$(realpath "${1:?usage: check-debs.sh <debs dir> [version]}")
version=${2:-$(ci/deb/version.sh)}
architecture=$(dpkg --print-architecture)
mapfile -t packages < <(dh_listpackages -a)
mapfile -t independent < <(dh_listpackages -i)
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

if [ "${PROFILE:-dist}" = dist ]; then
    echo "== every program keeps only its function names, in .gnu_debugdata"
    for package in "${packages[@]}"; do
        root=$(extract "$package")
        programs=0
        while IFS= read -r file; do
            [ "$(od -An -tx1 -N4 "$file" | tr -d ' ')" = 7f454c46 ] || continue
            programs=$((programs + 1))
            sections=$(readelf --wide --section-headers "$file")
            if grep -qE '\.debug_(info|line|str) ' <<< "$sections" || ! grep -q '\.gnu_debugdata' <<< "$sections"; then
                echo "${file#"$root"} in $package is not stripped to its .gnu_debugdata"
                exit 1
            fi
        done < <(find "$root" -type f)
        [ $programs -gt 0 ] || { echo "$package holds no program"; exit 1; }
    done
fi

echo "== every package is built for $architecture at $version, each with its debug symbols"
[ "$(find "$debs" -name "*_$architecture.deb" ! -name '*-dbgsym_*' | wc -l)" = "${#packages[@]}" ]
for package in "${packages[@]}"; do
    test -e "$debs/$package-dbgsym_${version}_$architecture.deb" || { echo "$package has no -dbgsym package"; exit 1; }
    deb="$debs/${package}_${version}_$architecture.deb"
    dpkg-deb --info "$deb"
    dpkg-deb --contents "$deb"
    [ "$(dpkg-deb --field "$deb" Package)" = "$package" ]
    [ "$(dpkg-deb --field "$deb" Architecture)" = "$architecture" ]
    [ "$(dpkg-deb --field "$deb" Version)" = "$version" ]
    [ -n "$(dpkg-deb --field "$deb" Section)" ] || { echo "$package has no section"; exit 1; }
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
field bande-greeter Provides | grep -qx x-display-manager
grep -qx 'ExecStart=/usr/sbin/greetd --config /usr/share/bande-greeter/greetd-config.toml' "$root/usr/lib/systemd/system/bande-greeter.service"
test ! -e "$root/usr/lib/systemd/system/greetd.service.d"
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

echo "== the keyring and its PAM module"
root=$(extract bande-secrets)
test ! -e "$root/usr/share/dbus-1/services/org.bande.Secrets.service"
test -x "$root/usr/libexec/bande-secrets"
test -x "$root/usr/libexec/bande-secrets-prompt"
grep -qx 'ListenStream=%t/bande-secrets/pam' "$root/usr/lib/systemd/user/bande-secrets.socket"
grep -qx 'BusName=org.freedesktop.secrets' "$root/usr/lib/systemd/user/bande-secrets.service"
field bande-secrets Provides | grep -qw org.freedesktop.secrets
grep -qx 'WantedBy=bande-session.target' "$root/usr/lib/systemd/user/bande-secrets.service"
[ -z "$(field bande-secrets Conflicts)" ]
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
grep -qx 'DesktopNames=X-Bande' "$session"
test -x "$root/usr/bin/bande-session"
sh -n "$root/usr/bin/bande-session"
grep -qx 'Type=notify' "$root/usr/lib/systemd/user/bande-compositor.service"
grep -qx 'BindsTo=graphical-session.target' "$root/usr/lib/systemd/user/bande-compositor.service"
grep -qx 'Wants=xdg-desktop-autostart.target' "$root/usr/lib/systemd/user/bande-session.target"

echo "== the portal backend's registration"
root=$(extract xdg-desktop-portal-bande)
portal="$root/usr/share/xdg-desktop-portal/portals/bande.portal"
service="$root/usr/share/dbus-1/services/org.freedesktop.impl.portal.desktop.bande.service"
grep -qx 'DBusName=org.freedesktop.impl.portal.desktop.bande' "$portal"
grep -qx 'Name=org.freedesktop.impl.portal.desktop.bande' "$service"
test -x "$root$(sed -n 's/^Exec=//p' "$service")"
grep -qx 'default=bande;' "$root/usr/share/xdg-desktop-portal/x-bande-portals.conf"

echo "== bande-desktop asks for its packages at $version"
all() { echo "$debs/$1_${version}_all.deb"; }
desktop() { dpkg-deb --field "$(all bande-desktop)" "$1"; }
desktop Depends | grep -qF "bande-compositor (>= $version)"
desktop Recommends | grep -qF "bande-files (>= $version)"
dpkg-deb --extract "$(all bande-desktop)" "$scratch/desktop"
grep -qx 'inode/directory=org.bande.Files.desktop;' "$scratch/desktop/usr/share/applications/x-bande-mimeapps.list"

echo "== bande-desktop brings a login, sound and the system around the desktop"
relation() { desktop "$1" | tr ',' '\n' | sed 's/^ *//' | grep -qxF "$2"; }
relation Depends "bande-greeter (>= $version) | x-display-manager"
for depended in pipewire wireplumber; do
    relation Depends "$depended"
done
for recommended in "bande-greeter (>= $version)" "bande-reporter (>= $version)" "bande-crash-reports (>= $version)" pipewire-pulse; do
    relation Recommends "$recommended"
done

echo "== bande-desktop recommends only packages built here at $version, the browser, or Debian's"
relation Recommends bande-browser
desktop Recommends | tr ',' '\n' | sed 's/^ *//' | grep '^bande-\|^libpam-bande-' | grep -vx bande-browser | while read -r package relation; do
    [ "$relation" = "(>= $version)" ] || { echo "bande-desktop recommends $package $relation"; exit 1; }
    printf '%s\n' "${packages[@]}" "${independent[@]}" | grep -qxF "$package" || { echo "$package is not built here"; exit 1; }
done

echo "== a language pack holds only catalogs, one per program"
for pack in "$debs"/bande-desktop-l10n-*_all.deb; do
    [ "$(dpkg-deb --field "$pack" Section)" = localization ]
    if dpkg-deb --contents "$pack" | awk '{print $6}' | grep -v -e '^\./usr/share/bande/l10n/' -e '^\./usr/share/doc/' -e '^\./$' -e '^\./usr/$' -e '^\./usr/share/$' -e '^\./usr/share/bande/$'; then
        echo "$pack holds more than catalogs"
        exit 1
    fi
done
contents=$(dpkg-deb --contents "$(all bande-desktop-l10n-sv)")
grep -q ' ./usr/share/bande/l10n/bande-settings/sv.mo$' <<< "$contents"
grep -q ' ./usr/share/bande/l10n/bande-mail/sv.mo$' <<< "$contents"
if grep -q ' ./usr/share/bande/l10n/bande-template/' <<< "$contents"; then echo "the Swedish pack holds the template's catalog"; exit 1; fi
echo "the $architecture and all packages passed"
