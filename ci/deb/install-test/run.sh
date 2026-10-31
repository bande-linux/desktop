#!/bin/bash
set -euo pipefail
export DEBIAN_FRONTEND=noninteractive
debs=$(realpath "${1:-/debs}")

apt-get update -qq
apt-get install -y -qq --no-install-recommends apt-utils > /dev/null
(cd "$debs" && apt-ftparchive packages . > Packages)
echo "deb [trusted=yes] file:$debs ./" > /etc/apt/sources.list.d/local.list
apt-get update -qq

echo "== bande-desktop resolves and installs from the packages alone"
ours=$(cd "$debs" && for deb in *.deb; do dpkg-deb --field "$deb" Package; done | grep -v '^bande-os$')
apt-get install -y -q --no-install-recommends $ours < /dev/null

echo "== every app bande-desktop recommends is in Debian testing"
apt-cache show bande-desktop | sed -n 's/^Recommends: //p' | tr ',' '\n' | sed 's/^ *//; s/[ (].*//' |
    while read -r recommended; do
        [ -n "$recommended" ] || continue
        candidate=$(apt-cache policy "$recommended" | sed -n 's/^ *Candidate: //p')
        [ -n "$candidate" ] && [ "$candidate" != "(none)" ] || { echo "$recommended cannot be installed"; exit 1; }
    done
version=$(dpkg-query -W -f '${Version}' bande-desktop)
for package in $(dpkg-query -W -f '${Package}\n' 'bande-*' xdg-desktop-portal-bande); do
    installed=$(dpkg-query -W -f '${db:Status-Status} ${Version}' "$package" 2>/dev/null || true)
    [ "$installed" = "installed $version" ] || [ -z "${installed%%not-installed*}" ] ||
        { echo "$package is $installed, bande-desktop is $version"; exit 1; }
done

echo "== no X server, XWayland or X session came along"
x11=$(dpkg-query -W -f '${db:Status-Status} ${Package}\n' | awk '$1 == "installed" { print $2 }' |
    grep -E '^(xserver-|xwayland$|xorg$|x11-session-utils$|xinit$)' || true)
[ -z "$x11" ] || { echo "installed: $x11"; exit 1; }

echo "== every packaged program is in place"
for program in $(dpkg -L $(dpkg-query -W -f '${Package}\n' 'bande-*' xdg-desktop-portal-bande) 2>/dev/null |
    grep -E '^/usr/(bin|libexec)/.'); do
    test -x "$program" || { echo "$program is not executable"; exit 1; }
done
test -e /usr/share/wayland-sessions/bande.desktop

echo "== greetd shows the Bande login window and is the display manager"
greetd_config=/usr/share/bande-greeter/greetd-config.toml
display_manager=/etc/systemd/system/display-manager.service
grep -qx "ExecStart=/usr/sbin/greetd --config $greetd_config" /usr/lib/systemd/system/greetd.service.d/bande.conf
test -x "$(sed -n 's/^command = "\([^ ]*\).*/\1/p' "$greetd_config")"
[ "$(readlink "$display_manager")" = /usr/lib/systemd/system/greetd.service ]
grep -qx /usr/sbin/greetd /etc/X11/default-display-manager
grep -q 'agreety' /etc/greetd/config.toml

echo "== a person's wallpaper reaches the login window and no one else"
pictures=/var/lib/bande-greeter/pictures
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
dpkg --purge --force-depends bande-greeter > /dev/null 2>&1
test ! -e /etc/X11/default-display-manager
test ! -e /usr/lib/systemd/system/greetd.service.d/bande.conf
test ! -e "$pictures"
ln -sfn /usr/lib/systemd/system/gdm3.service "$display_manager"
echo /usr/sbin/gdm3 > /etc/X11/default-display-manager
apt-get install -y -q --no-install-recommends bande-greeter < /dev/null > /dev/null
[ "$(readlink "$display_manager")" = /usr/lib/systemd/system/gdm3.service ]
grep -qx /usr/sbin/gdm3 /etc/X11/default-display-manager
rm "$display_manager" /etc/X11/default-display-manager
dpkg --purge --force-depends bande-greeter > /dev/null 2>&1
apt-get install -y -q --no-install-recommends bande-greeter < /dev/null > /dev/null
[ "$(readlink "$display_manager")" = /usr/lib/systemd/system/greetd.service ]
grep -qx /usr/sbin/greetd /etc/X11/default-display-manager

echo "== the installed compositor serves Wayland clients"
export XDG_RUNTIME_DIR=$(mktemp -d)
status=0
timeout 10 bande-compositor --headless > /tmp/compositor.out 2>&1 || status=$?
cat /tmp/compositor.out
[ "$status" = 124 ]
grep -q '^WAYLAND_DISPLAY=wayland-' /tmp/compositor.out

echo "== bande-os brings the system around the desktop"
apt-get install -y -q --no-install-recommends bande-os < /dev/null > /dev/null
for package in bande-desktop bande-greeter greetd network-manager pipewire wireplumber systemd-zram-generator; do
    [ "$(dpkg-query -W -f '${db:Status-Status}' "$package")" = installed ] || { echo "$package is missing"; exit 1; }
done
[ "$(dpkg-query -W -f '${Version}' bande-os)" = "$version" ]

echo "install test passed"
