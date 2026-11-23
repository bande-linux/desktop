#!/bin/bash
# quick.sh <out dir>: builds every binary package of this working tree, uncommitted changes
# included, into <out dir> for local testing, the way ci/deb/build.sh does (offline, with the
# pkg.bande.vendor profile, the same debian/ and file layout) but much faster: with the release
# profile (PROFILE picks another), without dh_auto_test, and in a tree and target directory kept
# in ~/.cache/bande-quick, so a rebuild compiles only what changed. The vendored crates are made
# again only when Cargo.lock changes. The version is ci/deb/version.sh's with +quickN after it,
# N counting the quick builds, so it is never taken for a release and each one upgrades the last.
set -euo pipefail
cd "$(dirname "$0")/../.."
out=$(realpath -m "${1:?usage: quick.sh <out dir>}")
cache=${XDG_CACHE_HOME:-$HOME/.cache}/bande-quick
tree=$cache/bande-desktop
mkdir -p "$tree" "$out"
exec 9> "$cache/lock"
flock -n 9 || { echo "quick.sh: another quick build is running" >&2; exit 1; }

export SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH:-$(git log -1 --format=%ct)}
count=$(($(cat "$cache/count" 2> /dev/null || echo 0) + 1))
version=$(ci/deb/version.sh)+quick$count
codename=$(sed -n 's/^VERSION_CODENAME=//p' /etc/os-release)
state="$(git rev-parse --short=7 HEAD)"
git diff --quiet HEAD || state="$state with uncommitted changes"

find "$tree" -mindepth 1 -maxdepth 1 ! -name vendor -exec rm -rf {} +
git ls-files -z --cached --others --exclude-standard |
    while IFS= read -r -d '' file; do
        if [ -e "$file" ] || [ -L "$file" ]; then printf '%s\0' "$file"; fi
    done |
    tar --create --null --files-from=- | tar --extract --directory="$tree"
if [ ! -d "$tree/vendor" ] || ! cmp -s Cargo.lock "$cache/vendor.lock"; then
    rm -rf "$tree/vendor" "$cache/vendor.lock"
    ci/deb/vendor.sh "$cache" quick
    tar -xJf "$cache/bande-desktop_quick.orig-vendor.tar.xz" -C "$tree"
    rm "$cache/bande-desktop_quick.orig-vendor.tar.xz"
    cp Cargo.lock "$cache/vendor.lock"
fi
ci/deb/changelog-entry.sh "$tree/debian/changelog" "$version" "$codename" "Quick local build of $state, not for release."

unset CARGO_BUILD_RUSTFLAGS RUSTFLAGS CFLAGS CXXFLAGS CPPFLAGS LDFLAGS
find "$cache" -maxdepth 1 -type f \( -name '*.deb' -o -name '*.changes' -o -name '*.buildinfo' \) -delete
echo "quick.sh: building $version with the ${PROFILE:-release} profile; dh_auto_test and the autopkgtests do not run"
(cd "$tree" && env -u SOURCE_DATE_EPOCH CARGO_NET_OFFLINE=true CARGO_TARGET_DIR="$cache/target" \
    PROFILE="${PROFILE:-release}" DEB_BUILD_PROFILES="pkg.bande.vendor${DEB_BUILD_PROFILES:+ $DEB_BUILD_PROFILES}" \
    DEB_BUILD_OPTIONS="nocheck${DEB_BUILD_OPTIONS:+ $DEB_BUILD_OPTIONS}" \
    dpkg-buildpackage --build=binary --no-sign)
echo "$count" > "$cache/count"
find "$cache" -maxdepth 1 -type f \( -name '*.deb' -o -name '*.changes' -o -name '*.buildinfo' \) -exec mv -t "$out" {} +
echo "quick.sh: $version is in $out"
echo "  check: ci/release/check-debs.sh $out $version && lintian --fail-on error,warning $out/*.changes"
echo "  install: sudo apt install $out/*_${version}_*.deb"
