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
target=${CARGO_TARGET_DIR:-target}
cargo build --release --locked --package=bande-l10n-tool
rm -rf "$target/l10n"
"$target/release/bande-l10n-tool" pack "$target/l10n"
ci/release/build-l10n.sh "$target/l10n" "$out" "$version" bande-desktop bande-desktop
ls -l "$out"
deb() { echo "$out/$1_$2_all.deb"; }
field() { dpkg-deb --field "$(deb "$1" "$version")" "$2"; }
scratch=$(mktemp -d)
trap 'rm -rf "$scratch"' EXIT

echo "== bande-desktop pins its packages at $version"
[ "$(field bande-desktop Version)" = "$version" ]
field bande-desktop Depends | grep -qF "bande-compositor (= $version)"
field bande-desktop Recommends | grep -qF "bande-files (= $version)"
dpkg-deb --extract "$(deb bande-desktop "$version")" "$scratch/desktop"
grep -qx 'inode/directory=org.bande.Files.desktop;' "$scratch/desktop/usr/share/applications/bande-mimeapps.list"

echo "== bande-desktop brings a login, sound and the system around the desktop"
relation() { field bande-desktop "$1" | tr ',' '\n' | sed 's/^ *//' | grep -qxF "$2"; }
relation Depends "bande-greeter (= $version) | x-display-manager"
for depended in pipewire wireplumber pipewire-pulse; do
    relation Depends "$depended"
done
for recommended in "bande-greeter (= $version)" bande-apt-source systemd-zram-generator; do
    relation Recommends "$recommended"
done

echo "== bande-desktop recommends each default app unversioned, none built here"
while read -r package; do
    relation Recommends "$package" || { echo "bande-desktop does not recommend $package"; exit 1; }
    ! grep -qxF "$package" ci/release/packages || { echo "$package is built here, not a default app"; exit 1; }
done < <(grep -E '^[a-z0-9]' packaging/bande-desktop/default-apps)

echo "== a language pack holds only catalogs, one per program"
for pack in "$out"/bande-desktop-l10n-*_all.deb; do
    [ "$(dpkg-deb --field "$pack" Section)" = localization ]
    ! dpkg-deb --contents "$pack" | awk '{print $6}' | grep -v -e '^\./usr/share/bande/l10n/' -e '^\./usr/share/doc/' -e '^\./$' -e '^\./usr/$' -e '^\./usr/share/$' -e '^\./usr/share/bande/$'
done
pack=$(deb bande-desktop-l10n-sv "$version")
dpkg-deb --contents "$pack" | grep -q ' ./usr/share/bande/l10n/bande-settings/sv.mo$'

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
