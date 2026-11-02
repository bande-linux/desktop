#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/../.."

version=${DEB_VERSION:-$(ci/deb/version.sh)}
target=${CARGO_TARGET_DIR:-target}
mapfile -t packages < ci/release/packages
mapfile -t bundled < ci/release/bundled
export RUSTFLAGS="${RUSTFLAGS:-} --remap-path-prefix=$PWD=/build --remap-path-prefix=${CARGO_HOME:-$HOME/.cargo}=/cargo --remap-path-prefix=${RUSTUP_HOME:-$HOME/.rustup}=/rustup"

echo "Building ${#packages[@]} packages $version for $(dpkg --print-architecture)"
rm -rf "$target/debian"
cargo build --profile dist --locked "${packages[@]/#/--package=}" "${bundled[@]/#/--package=}"
architecture=$(dpkg --print-architecture)
for package in "${packages[@]}"; do
    variant=()
    if grep -qx "$package" ci/release/arch-variants; then
        variant=(--variant "$architecture")
    fi
    cargo deb --profile dist --no-build --locked --package "$package" "${variant[@]}" --deb-version "$version"
done
ls -l "$target/debian"
