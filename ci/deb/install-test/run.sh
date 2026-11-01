#!/bin/bash
set -euo pipefail
export DEBIAN_FRONTEND=noninteractive
debs=$(realpath "${1:-/debs}")

apt-get update -qq
apt-get install -y -qq --no-install-recommends apt-utils > /dev/null
(cd "$debs" && apt-ftparchive packages . > Packages)
echo "deb [trusted=yes] file:$debs ./" > /etc/apt/sources.list.d/local.list
apt-get update -qq

echo "== bande-desktop alone brings a login window and sound"
apt-get install -y -q --no-install-recommends bande-desktop < /dev/null
for package in bande-greeter greetd pipewire-audio wireplumber; do
    [ "$(dpkg-query -W -f '${db:Status-Status}' "$package")" = installed ] || { echo "$package is missing"; exit 1; }
done

echo "== every package resolves and installs from the packages alone"
ours=$(cd "$debs" && for deb in *.deb; do dpkg-deb --field "$deb" Package; done)
apt-get install -y -q --no-install-recommends $ours < /dev/null

echo "== every package bande-desktop recommends is in Debian testing or built here"
default_apps="$(dirname "$0")/../../../packaging/bande-desktop/default-apps"
from_the_archive=" bande-apt-source $(awk '/^\[/ { apps = $0 == "[apps]"; next } apps && /^[a-z0-9]/ { print $1 }' "$default_apps" | xargs) "
apt-cache show bande-desktop | sed -n 's/^Recommends: //p' | tr ',' '\n' | sed 's/^ *//; s/[ (].*//' |
    while read -r recommended; do
        [ -n "$recommended" ] && [[ $from_the_archive != *" $recommended "* ]] || continue
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
greetd_dropin=/usr/lib/systemd/system/greetd.service.d/bande.conf
grep -qx "ExecStart=/usr/sbin/greetd --config $greetd_config" "$greetd_dropin"
test -x "$(sed -n 's/^command = "\([^ ]*\).*/\1/p' "$greetd_config")"
default_display_manager() {
    grep -qx "/usr/sbin/$1" /etc/X11/default-display-manager
    [ "$(basename "$(readlink /etc/systemd/system/display-manager.service)")" = "$1.service" ]
    [ "$(echo get shared/default-x-display-manager | debconf-communicate)" = "0 $1" ]
}
default_display_manager greetd
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
dpkg --purge --force-depends bande-greeter > /dev/null
test ! -e "$greetd_dropin"
test ! -e "$pictures"
default_display_manager greetd

echo "== the installed compositor serves Wayland clients"
export XDG_RUNTIME_DIR=$(mktemp -d)
status=0
timeout 10 bande-compositor --headless > /tmp/compositor.out 2>&1 || status=$?
cat /tmp/compositor.out
[ "$status" = 124 ]
grep -q '^WAYLAND_DISPLAY=wayland-' /tmp/compositor.out

echo "== next to another display manager, greetd is offered, not imposed"
dpkg --purge bande-desktop greetd > /dev/null 2>&1
apt-get install -y -q --no-install-recommends lightdm bsdutils < /dev/null > /dev/null
default_display_manager lightdm
apt-get install -y -q --no-install-recommends bande-desktop < /dev/null > /dev/null
[ "$(dpkg-query -W -f '${db:Status-Status}' greetd 2>/dev/null || true)" != installed ]
apt-get install -y -q --no-install-recommends bande-greeter < /dev/null > /dev/null
default_display_manager lightdm
echo metaget shared/default-x-display-manager choices | debconf-communicate | grep -qw greetd
echo greetd | DEBIAN_FRONTEND=teletype script -qec 'dpkg-reconfigure greetd' /dev/null > /dev/null
default_display_manager greetd
apt-get remove -y -q bande-greeter greetd < /dev/null > /dev/null
default_display_manager lightdm

echo "install test passed"
