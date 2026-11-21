#!/bin/bash
# build.sh <dsc> <out dir> [dpkg-buildpackage option...]: builds the binary packages of a source
# package from ci/deb/source.sh for the Debian release this runs on, offline and with the
# pkg.bande.vendor profile, at the version from ci/deb/version.sh (the release's ~debNN suffix,
# in a debian/changelog entry for that release), into <out dir>.
set -euo pipefail
cd "$(dirname "$0")/../.."
dsc=$(realpath "${1:?usage: build.sh <dsc> <out dir> [dpkg-buildpackage option...]}")
out=$(realpath -m "${2:?usage: build.sh <dsc> <out dir> [dpkg-buildpackage option...]}")
shift 2
version=$(ci/deb/version.sh)
codename=$(. /etc/os-release && echo "$VERSION_CODENAME")
export SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH:-$(git log -1 --format=%ct)}
scratch=$(mktemp -d)
trap 'rm -rf "$scratch"' EXIT
dpkg-source --extract "$dsc" "$scratch/source"
ci/deb/changelog-entry.sh "$scratch/source/debian/changelog" "$version" "$codename" "Build for Debian $codename."
(cd "$scratch/source" && CARGO_NET_OFFLINE=true dpkg-buildpackage --unsigned-source --unsigned-changes \
    --build-profiles=pkg.bande.vendor "$@")
mkdir -p "$out"
find "$scratch" -maxdepth 1 -type f \( -name '*.deb' -o -name '*.ddeb' -o -name '*.changes' -o -name '*.buildinfo' \) \
    -exec mv -t "$out" {} +
