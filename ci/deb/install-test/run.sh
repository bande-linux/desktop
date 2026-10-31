#!/bin/bash
set -euo pipefail
export DEBIAN_FRONTEND=noninteractive
debs=$(realpath "${1:-/debs}")

apt-get update -qq
apt-get install -y -qq --no-install-recommends apt-utils > /dev/null
(cd "$debs" && apt-ftparchive packages . > Packages)
echo "deb [trusted=yes] file:$debs ./" > /etc/apt/sources.list.d/local.list
apt-get update -qq

echo "== unbound-desktop resolves and installs from the packages alone"
ours=$(cd "$debs" && for deb in *.deb; do dpkg-deb --field "$deb" Package; done | grep -v -e apt-source -e archive-keyring -e '^unbound-os$')
apt-get install -y -q --no-install-recommends $ours < /dev/null

echo "== every app unbound-desktop recommends is in Debian testing"
apt-cache show unbound-desktop | sed -n 's/^Recommends: //p' | tr ',' '\n' | sed 's/^ *//; s/[ (].*//' |
    while read -r recommended; do
        [ -n "$recommended" ] || continue
        candidate=$(apt-cache policy "$recommended" | sed -n 's/^ *Candidate: //p')
        [ -n "$candidate" ] && [ "$candidate" != "(none)" ] || { echo "$recommended cannot be installed"; exit 1; }
    done
version=$(dpkg-query -W -f '${Version}' unbound-desktop)
for package in $(dpkg-query -W -f '${Package}\n' 'unbound-*' xdg-desktop-portal-unbound | grep -v -e apt-source -e archive-keyring); do
    installed=$(dpkg-query -W -f '${db:Status-Status} ${Version}' "$package" 2>/dev/null || true)
    [ "$installed" = "installed $version" ] || [ -z "${installed%%not-installed*}" ] ||
        { echo "$package is $installed, unbound-desktop is $version"; exit 1; }
done

echo "== no X server, XWayland or X session came along"
x11=$(dpkg-query -W -f '${db:Status-Status} ${Package}\n' | awk '$1 == "installed" { print $2 }' |
    grep -E '^(xserver-|xwayland$|xorg$|x11-session-utils$|xinit$)' || true)
[ -z "$x11" ] || { echo "installed: $x11"; exit 1; }

echo "== every packaged program is in place"
for program in $(dpkg -L $(dpkg-query -W -f '${Package}\n' 'unbound-*' xdg-desktop-portal-unbound) 2>/dev/null |
    grep -E '^/usr/(bin|libexec)/.'); do
    test -x "$program" || { echo "$program is not executable"; exit 1; }
done
test -e /usr/share/wayland-sessions/unbound.desktop

echo "== greetd shows the Unbound login window and is the display manager"
greetd_config=/usr/share/unbound-greeter/greetd-config.toml
display_manager=/etc/systemd/system/display-manager.service
grep -qx "ExecStart=/usr/sbin/greetd --config $greetd_config" /usr/lib/systemd/system/greetd.service.d/unbound.conf
test -x "$(sed -n 's/^command = "\([^ ]*\).*/\1/p' "$greetd_config")"
[ "$(readlink "$display_manager")" = /usr/lib/systemd/system/greetd.service ]
grep -qx /usr/sbin/greetd /etc/X11/default-display-manager
grep -q 'agreety' /etc/greetd/config.toml

echo "== a person's wallpaper reaches the login window and no one else"
pictures=/var/lib/unbound-greeter/pictures
[ "$(stat -c '%a %U %G' "$pictures")" = "3733 root _greetd" ]
useradd --create-home ada
useradd --create-home alan
runuser -u ada -- sh -c "umask 077; echo picture > $pictures/\$(id -u); chmod 640 $pictures/\$(id -u)"
[ "$(runuser -u _greetd -- cat "$pictures/$(id -u ada)")" = picture ]
refused() {
    if "$@" > /dev/null 2>&1; then
        echo "allowed: $*"
        exit 1
    fi
}
refused runuser -u alan -- cat "$pictures/$(id -u ada)"
refused runuser -u alan -- ls "$pictures"
refused runuser -u alan -- rm -f "$pictures/$(id -u ada)"
test -e "$pictures/$(id -u ada)"
dpkg --purge --force-depends unbound-greeter > /dev/null 2>&1
test ! -e /etc/X11/default-display-manager
test ! -e /usr/lib/systemd/system/greetd.service.d/unbound.conf
test ! -e "$pictures"
ln -sfn /usr/lib/systemd/system/gdm3.service "$display_manager"
echo /usr/sbin/gdm3 > /etc/X11/default-display-manager
apt-get install -y -q --no-install-recommends unbound-greeter < /dev/null > /dev/null
[ "$(readlink "$display_manager")" = /usr/lib/systemd/system/gdm3.service ]
grep -qx /usr/sbin/gdm3 /etc/X11/default-display-manager
rm "$display_manager" /etc/X11/default-display-manager
dpkg --purge --force-depends unbound-greeter > /dev/null 2>&1
apt-get install -y -q --no-install-recommends unbound-greeter < /dev/null > /dev/null
[ "$(readlink "$display_manager")" = /usr/lib/systemd/system/greetd.service ]
grep -qx /usr/sbin/greetd /etc/X11/default-display-manager

echo "== the installed compositor serves Wayland clients"
export XDG_RUNTIME_DIR=$(mktemp -d)
status=0
timeout 10 unbound-compositor --headless > /tmp/compositor.out 2>&1 || status=$?
cat /tmp/compositor.out
[ "$status" = 124 ]
grep -q '^WAYLAND_DISPLAY=wayland-' /tmp/compositor.out

echo "== unbound-os brings the system around the desktop"
apt-get install -y -q --no-install-recommends unbound-os < /dev/null > /dev/null
for package in unbound-desktop unbound-greeter unbound-apt-source greetd network-manager pipewire wireplumber systemd-zram-generator; do
    [ "$(dpkg-query -W -f '${db:Status-Status}' "$package")" = installed ] || { echo "$package is missing"; exit 1; }
done
[ "$(dpkg-query -W -f '${Version}' unbound-os)" = "$version" ]

echo "== the APT source follows the rolling suite and allows only Unbound's packages"
rm /etc/apt/sources.list.d/local.list
grep -qx 'Suites: rolling' /etc/apt/sources.list.d/unbound.sources
grep -qx 'Components: main apple' /etc/apt/sources.list.d/unbound.sources
grep -qx 'Signed-By: /usr/share/keyrings/unbound-archive-keyring.pgp' /etc/apt/sources.list.d/unbound.sources
test -s /usr/share/keyrings/unbound-archive-keyring.pgp
grep -qx 'Pin: origin unbound-linux.github.io' /etc/apt/preferences.d/unbound.pref
grep -qx 'Package: unbound-\* xdg-desktop-portal-unbound linux-\*-apple-\* unbound-os-apple' /etc/apt/preferences.d/unbound.pref
echo "install test passed"
