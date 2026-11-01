#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/../.."

out=$(realpath -m "${1:-${CARGO_TARGET_DIR:-target}/debian-all}")
version=${DEB_VERSION:-$(ci/deb/version.sh)}
rm -rf "$out"
mkdir -p "$out"
(cd packaging/bande-desktop &&
    DEB_VERSION_OVERRIDE=$version dpkg-buildpackage -b -us -uc &&
    DEB_VERSION_OVERRIDE=$version debian/rules clean)
mv "packaging/bande-desktop_${version}_all.deb" "$out/"
rm -f packaging/*.buildinfo packaging/*.changes
ls -l "$out"
deb() { echo "$out/$1_$2_all.deb"; }
field() { dpkg-deb --field "$(deb "$1" "$version")" "$2"; }
scratch=$(mktemp -d)
trap 'rm -rf "$scratch"' EXIT

echo "== bande-desktop pins its packages at $version"
[ "$(field bande-desktop Version)" = "$version" ]
field bande-desktop Depends | grep -qF "bande-compositor (= $version)"
field bande-desktop Recommends | grep -qF "bande-monitor (= $version)"
dpkg-deb --extract "$(deb bande-desktop "$version")" "$scratch/desktop"
grep -qx 'application/pdf=org.gnome.Papers.desktop;' "$scratch/desktop/usr/share/applications/bande-mimeapps.list"

echo "== bande-desktop brings a login, sound and the system around the desktop"
relation() { field bande-desktop "$1" | tr ',' '\n' | sed 's/^ *//' | grep -qxF "$2"; }
relation Depends "bande-greeter (= $version) | x-display-manager"
relation Depends pipewire-audio
for recommended in "bande-greeter (= $version)" network-manager bande-apt-source systemd-zram-generator; do
    relation Recommends "$recommended"
done

echo "== bande-desktop recommends or suggests each default app unversioned, none built here"
while read -r package _ wanted; do
    relation "${wanted^}" "$package" || { echo "bande-desktop does not $wanted $package"; exit 1; }
    ! grep -qxF "$package" ci/release/packages || { echo "$package is built here, not a default app"; exit 1; }
done < <(grep -E '^[a-z0-9]' packaging/bande-desktop/default-apps)

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
