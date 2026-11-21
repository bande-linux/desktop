#!/bin/bash
# source.sh <out dir>: the Debian source package of HEAD in <out dir>: the .dsc, the upstream
# tarball (ci/deb/orig.sh), the vendored crates (ci/deb/vendor.sh) and debian/ at the version
# from ci/deb/version.sh --source; a snapshot gets its own debian/changelog entry.
set -euo pipefail
cd "$(dirname "$0")/../.."
out=$(realpath -m "${1:?usage: source.sh <out dir>}")
version=$(ci/deb/version.sh --source)
upstream=${version%-*}
export SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH:-$(git log -1 --format=%ct)}
mkdir -p "$out"
tree=$out/bande-desktop-$upstream
rm -rf "$tree"
ci/deb/orig.sh "$out" "$upstream"
ci/deb/vendor.sh "$out" "$upstream"
tar -xJf "$out/bande-desktop_$upstream.orig.tar.xz" -C "$out"
tar -xJf "$out/bande-desktop_$upstream.orig-vendor.tar.xz" -C "$tree"
git archive --format=tar HEAD debian | tar -x -C "$tree"
ci/deb/changelog-entry.sh "$tree/debian/changelog" "$version" unstable "Snapshot of $(git rev-parse --short=7 HEAD)."
(cd "$out" && dpkg-source --build "bande-desktop-$upstream")
rm -rf "$tree"
ls "$out/bande-desktop_$version.dsc"
