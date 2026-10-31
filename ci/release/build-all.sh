#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/../.."

out=$(realpath -m "${1:-${CARGO_TARGET_DIR:-target}/debian-all}")
version=${DEB_VERSION:-$(ci/deb/version.sh)}
rm -rf "$out"
mkdir -p "$out"
for source in unbound-apt-source unbound-desktop unbound-os; do
    (cd "packaging/$source" &&
        DEB_VERSION_OVERRIDE=$version dpkg-buildpackage -b -us -uc &&
        DEB_VERSION_OVERRIDE=$version debian/rules clean)
    mv packaging/*.deb "$out/"
    rm -f packaging/*.buildinfo packaging/*.changes
done
ls -l "$out"
deb() { echo "$out/$1_$2_all.deb"; }
field() { dpkg-deb --field "$(deb "$1" "$version")" "$2"; }
scratch=$(mktemp -d)
trap 'rm -rf "$scratch"' EXIT

echo "== unbound-desktop pins its packages at $version"
[ "$(field unbound-desktop Version)" = "$version" ]
field unbound-desktop Depends | grep -qF "unbound-compositor (= $version)"
field unbound-desktop Recommends | grep -qF "unbound-monitor (= $version)"
dpkg-deb --extract "$(deb unbound-desktop "$version")" "$scratch/desktop"
grep -qx 'application/pdf=org.gnome.Papers.desktop;' "$scratch/desktop/usr/share/applications/unbound-mimeapps.list"

echo "== unbound-os pulls in the desktop and the system around it, and not the other way round"
[ "$(field unbound-os Version)" = "$version" ]
for dependency in "unbound-desktop (= $version)" "unbound-greeter (= $version)" unbound-apt-source greetd \
    network-manager pipewire wireplumber systemd-zram-generator; do
    field unbound-os Depends | tr ',' '\n' | sed 's/^ *//' | grep -qxF "$dependency"
done
for relation in Depends Pre-Depends Recommends Suggests; do
    [ -z "$(field unbound-desktop "$relation" | grep -w unbound-os || true)" ]
done

echo "== the APT source follows the rolling suite with only Unbound's packages allowed"
apt_source=$(ls "$out"/unbound-apt-source_*_all.deb)
dpkg-deb --extract "$apt_source" "$scratch/source"
sources="$scratch/source/etc/apt/sources.list.d/unbound.sources"
grep -qx 'URIs: https://unbound-linux.github.io/apt' "$sources"
grep -qx 'Suites: rolling' "$sources"
grep -qx 'Components: main apple' "$sources"
grep -qx 'Signed-By: /usr/share/keyrings/unbound-archive-keyring.pgp' "$sources"
dpkg-deb --extract "$(ls "$out"/unbound-archive-keyring_*_all.deb)" "$scratch/keyring"
grep -qx 'Package: unbound-\* xdg-desktop-portal-unbound linux-\*-apple-\* unbound-os-apple' \
    "$scratch/keyring/etc/apt/preferences.d/unbound.pref"
gpg --batch --with-colons --show-keys "$scratch/keyring/usr/share/keyrings/unbound-archive-keyring.pgp" |
    grep -qx 'fpr:::::::::6287DE66459929AA1C01E146495A6C1982647E05:'

echo "== lintian"
lintian=(lintian --fail-on "error,warning")
if [ "$(id -u)" = 0 ]; then
    chmod 755 "$scratch"
    cp "$out"/*.deb "$scratch/"
    chmod 644 "$scratch"/*.deb
    runuser -u nobody -- "${lintian[@]}" "$scratch"/*.deb
else
    "${lintian[@]}" "$out"/*.deb
fi
echo "the all packages passed"
