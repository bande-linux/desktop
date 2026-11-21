#!/bin/bash
# deb-assets.sh <dir> <version> <package>...: the generated package data cargo-deb ships, in
# <dir>/<package>: the copyright file from ci/copyright.py, the changelog and manual pages from
# ci/release/deb-docs.py and each metainfo with its release (the version and the date of the
# crate's last commit).
set -euo pipefail
cd "$(dirname "$0")/../.."
dir=${1:?usage: deb-assets.sh <dir> <version> <package>...}
version=${2:?usage: deb-assets.sh <dir> <version> <package>...}
shift 2
manifests=$(cargo metadata --no-deps --format-version 1 --locked | jq -r '.packages[] | "\(.name) \(.manifest_path)"')
for package in "$@"; do
    rm -rf "${dir:?}/$package"
    mkdir -p "$dir/$package"
    bundle=()
    [ "$package" != bande-secrets ] || bundle=(bande-secrets-prompt)
    ci/copyright.py "$package" "${bundle[@]}" > "$dir/$package/copyright"
    ci/release/deb-docs.py "$package" "$version" "$dir/$package"
    crate=$(dirname "$(awk -v name="$package" '$1 == name { print $2 }' <<< "$manifests")")
    released=$(TZ=UTC git log -1 --format=%cd --date=format-local:%Y-%m-%d -- "$crate")
    for metainfo in "$crate"/data/*.metainfo.xml; do
        [ -e "$metainfo" ] || continue
        sed "s|^</component>$|  <releases>\n    <release version=\"${version%%[~+]*}\" date=\"$released\"/>\n  </releases>\n</component>|" \
            "$metainfo" > "$dir/$package/$(basename "$metainfo")"
    done
done
