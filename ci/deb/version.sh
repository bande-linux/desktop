#!/bin/bash
# version.sh [--source] [repo]: the Debian version of this build, ending in the suite suffix ~debNN
# (ci/deb/debian-release.sh); with --source, the version of the source package, which has none.
# A vX.Y.Z or vX.Y.Z-R tag builds the debian/changelog version, which must be X.Y.Z-R (R is 1
# for vX.Y.Z) with X.Y.Z the Cargo.toml version; an l10n-YYYYMMDD[.N] tag builds the changelog
# version with +l10nYYYYMMDD[.N] after the suffix; any other build is a snapshot,
# <last release>+git<date>.<commits since it>.<sha>-1.
set -euo pipefail

source=0
[ "${1:-}" != --source ] || { source=1; shift; }
repo=${1:-.}
suffix=$("$(dirname "$(realpath "$0")")/debian-release.sh")
[ $source = 0 ] || suffix=
declared=$(awk '/^\[/ { section = $0 } section == "[workspace.package]" && $1 == "version" { gsub(/"/, "", $3); print $3; exit }' "$repo/Cargo.toml")
[ -n "$declared" ] || { echo "version.sh: Cargo.toml declares no workspace version" >&2; exit 1; }
changelog=$(dpkg-parsechangelog -l "$repo/debian/changelog" -S Version)

if [ "${GITHUB_REF_TYPE:-}" = tag ]; then
    tag=${GITHUB_REF_NAME:?}
    if [[ $tag =~ ^l10n-([0-9]{8}(\.[0-9]+)?)$ ]]; then
        echo "$changelog$suffix+l10n${BASH_REMATCH[1]}"
        exit 0
    fi
    [[ $tag =~ ^v([0-9]+\.[0-9]+\.[0-9]+)(-([0-9]+))?$ ]] ||
        { echo "version.sh: $tag is not a vX.Y.Z or vX.Y.Z-R tag" >&2; exit 1; }
    [ "${BASH_REMATCH[1]}" = "$declared" ] ||
        { echo "version.sh: tag $tag but Cargo.toml says $declared" >&2; exit 1; }
    [ "${BASH_REMATCH[1]}-${BASH_REMATCH[3]:-1}" = "$changelog" ] ||
        { echo "version.sh: tag $tag but debian/changelog says $changelog" >&2; exit 1; }
    echo "$changelog$suffix"
    exit 0
fi

last=$(git -C "$repo" describe --tags --abbrev=0 --match 'v[0-9]*.[0-9]*.[0-9]*' HEAD 2>/dev/null || true)
if [ -n "$last" ]; then
    base=${last#v}
    base=${base%-*}
    count=$(git -C "$repo" rev-list --count "$last..HEAD")
else
    base=0.0.0
    count=$(git -C "$repo" rev-list --count HEAD)
fi
date=$(TZ=UTC git -C "$repo" log -1 --format=%cd --date=format-local:%Y%m%d)
sha=$(git -C "$repo" rev-parse --short=7 HEAD)
echo "$base+git$date.$count.$sha-1$suffix"
