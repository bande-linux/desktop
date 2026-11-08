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
    --package=bande-l10n-tool
rm -rf "$target/$profile/l10n"
"$target/$profile/bande-l10n-tool" pack "$target/$profile/l10n"
architecture=$(dpkg --print-architecture)
if [ "$profile" = dist ]; then
    ci/release/symbols.sh "$target/$profile" "$target/debian-symbols" "$version"
    strip=(--no-strip)
fi
for package in "${packages[@]}"; do
    variant=()
    if grep -qx "$package" ci/release/arch-variants; then
        variant=(--variant "$architecture")
    fi
    cargo deb --profile "$profile" --no-build --locked --package "$package" "${variant[@]}" "${compression[@]}" "${strip[@]}" --deb-version "$version"
done
ls -l "$target/debian"
