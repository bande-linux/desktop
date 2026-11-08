#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/../.."

version=${DEB_VERSION:-$(ci/deb/version.sh)}
target=${CARGO_TARGET_DIR:-target}
mapfile -t packages < ci/release/packages
mapfile -t bundled < ci/release/bundled
profile=${VERIFY_PROFILE:-dist}
compression=()
strip=()
if [ "$profile" = dist ]; then
    export CARGO_BUILD_JOBS=${DIST_BUILD_JOBS:-2}
else
    compression=(--fast)
fi

echo "Building ${#packages[@]} packages $version for $(dpkg --print-architecture)"
rm -rf "$target/debian"
cargo build --profile "$profile" --locked "${packages[@]/#/--package=}" "${bundled[@]/#/--package=}" \
    --package=bande-l10n-tool --message-format=json-render-diagnostics |
    jq -r 'select(.reason == "compiler-artifact") | .executable // (select(.target.kind | index("cdylib")) | .filenames[] | select(endswith(".so")))' > "$target/shipped"
architecture=$(dpkg --print-architecture)
built=$target/$profile
[ "$profile" = dev ] && built=$target/debug
rm -rf "$built/cursors" "$built/l10n"
"$built/bande-cursor-theme" "$built/cursors/Bande-Cursors"
"$built/bande-l10n-tool" pack "$built/l10n"
packaged=$target
if [ "$profile" = dist ]; then
    packaged=$target/stripped
    ci/release/symbols.sh "$target/shipped" "$packaged/$profile" "$target/debian-symbols" "$version"
    cp -a "$built/cursors" "$built/l10n" "$packaged/$profile/"
    strip=(--no-strip)
fi
for package in "${packages[@]}"; do
    variant=()
    if grep -qx "$package" ci/release/arch-variants; then
        variant=(--variant "$architecture")
    fi
    CARGO_TARGET_DIR=$packaged cargo deb --output "$target/debian" --profile "$profile" --no-build --locked --package "$package" "${variant[@]}" "${compression[@]}" "${strip[@]}" --deb-version "$version"
done
ls -l "$target/debian"
