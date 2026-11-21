#!/bin/bash
# build-app.sh <app> <out dir> [profile]: the build of one app, the same in CI and in the local
# gate: ci/check-app.sh, the package version (ci/version.sh), ci/check.sh, then the binaries in
# the Cargo profile (default release; the gate's `gate` profile is release without its slow
# optimisations), the generated package data in <target>/<profile>/deb-assets/bande-<app> (the
# copyright file from ci/copyright.py, the changelog and manual pages from ci/deb-docs.py, the
# metainfo with its release: the Cargo version and the date of the app's last commit),
# bande-<app>_<version>_<arch>.deb with its name, architecture and version checked and lintian
# clean, its language packs (bande-l10n-tool debs) and ci/smoke.sh on them all. Leaves the .debs
# in <out dir>.
set -euo pipefail
cd "$(dirname "$0")/.."
app=${1:?usage: build-app.sh <app> <out dir> [profile]} out=$(realpath -m "${2:?usage: build-app.sh <app> <out dir> [profile]}")
profile=${3:-release}
dir=apps/$app name=bande-$app
arch=$(dpkg --print-architecture)
export CARGO_TERM_COLOR=never

cargo metadata --no-deps --format-version 1 | jq -e --arg name "$name" --arg dir "$PWD/$dir/Cargo.toml" \
    'any(.packages[]; .name == $name and .manifest_path == $dir)' > /dev/null ||
    { echo "build-app: apps/$app must be the package $name" >&2; exit 1; }
ci/check-app.sh "$dir"
version=$(ci/version.sh "$app")
echo "build-app: $name $version for $arch, $profile profile"
ci/check.sh "$name"
cargo build --profile "$profile" --locked -q -p "$name"
target=${CARGO_TARGET_DIR:-target}
rm -rf "$target/$profile/l10n"
ci/l10n-tool.sh pack "$target/$profile/l10n"
assets=$target/$profile/deb-assets/$name
rm -rf "$assets"
mkdir -p "$assets"
ci/copyright.py "$name" > "$assets/copyright"
ci/deb-docs.py "$name" "$version" "$assets"
released=$(TZ=UTC git log -1 --format=%cd --date=format-local:%Y-%m-%d -- "$dir")
for metainfo in "$dir"/data/*.metainfo.xml; do
    sed "s|^</component>$|  <releases>\n    <release version=\"${version%%[~+]*}\" date=\"$released\"/>\n  </releases>\n</component>|" \
        "$metainfo" > "$assets/$(basename "$metainfo")"
done
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
deb=$(cargo deb --no-build --locked -q --profile "$profile" -p "$name" --deb-version "$version" -o "$work/")
[ "$(dpkg-deb -f "$deb" Package)" = "$name" ] && [ "$(dpkg-deb -f "$deb" Architecture)" = "$arch" ] &&
    [ "$(dpkg-deb -f "$deb" Version)" = "$version" ] ||
    { echo "build-app: $deb has the wrong name, architecture or version" >&2; exit 1; }
lintian=(lintian --fail-on "error,warning")
if [ "$(id -u)" = 0 ]; then
    chmod 755 "$work"
    chmod 644 "$deb"
    runuser -u nobody -- "${lintian[@]}" "$deb"
else
    "${lintian[@]}" "$deb"
fi
mkdir -p "$out"
cp "$deb" "$out/"
ci/l10n-tool.sh debs "$target/$profile/l10n" "$out" "$version" "$name" ci/copyright
ci/smoke.sh "$deb" "$out/$name-l10n-"*"_${version}_all.deb"
echo "build-app: $(basename "$deb") passed"
