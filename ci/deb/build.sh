#!/bin/bash
# build.sh <dsc> <out dir> [dpkg-buildpackage option...]: builds the binary packages of a source
# package from ci/deb/source.sh for the Debian release this runs on, offline and with the
# pkg.bande.vendor profile (added to $DEB_BUILD_PROFILES), at the version from ci/deb/version.sh
# (the release's ~debNN suffix, in a debian/changelog entry for that release), into <out dir>.
# PROFILE picks the cargo profile (default dist) and CARGO_TARGET_DIR may keep the build.
set -euo pipefail
cd "$(dirname "$0")/../.."
dsc=$(realpath "${1:?usage: build.sh <dsc> <out dir> [dpkg-buildpackage option...]}")
out=$(realpath -m "${2:?usage: build.sh <dsc> <out dir> [dpkg-buildpackage option...]}")
shift 2
version=$(ci/deb/version.sh)
codename=$(sed -n 's/^VERSION_CODENAME=//p' /etc/os-release)
unset CARGO_BUILD_RUSTFLAGS RUSTFLAGS CFLAGS CXXFLAGS CPPFLAGS LDFLAGS
export DEB_BUILD_PROFILES="pkg.bande.vendor${DEB_BUILD_PROFILES:+ $DEB_BUILD_PROFILES}"
export SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH:-$(git log -1 --format=%ct)}
mkdir -p "$out"
scratch=$(mktemp -d -p "$out")
trap 'rm -rf "$scratch"' EXIT
dpkg-source --extract "$dsc" "$scratch/source"
ci/deb/changelog-entry.sh "$scratch/source/debian/changelog" "$version" "$codename" "Build for Debian $codename."
(cd "$scratch/source" && env -u SOURCE_DATE_EPOCH CARGO_NET_OFFLINE=true dpkg-buildpackage --unsigned-source --unsigned-changes "$@")
find "$scratch" -maxdepth 1 -type f \( -name '*.deb' -o -name '*.changes' -o -name '*.buildinfo' \) \
    -exec mv -t "$out" {} +
