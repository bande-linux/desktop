#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/../.."

version=${DEB_VERSION:-$(ci/deb/version.sh)}
target=${CARGO_TARGET_DIR:-target}
mapfile -t packages < ci/release/packages
export RUSTFLAGS="${RUSTFLAGS:-} --remap-path-prefix=$PWD=/build --remap-path-prefix=${CARGO_HOME:-$HOME/.cargo}=/cargo --remap-path-prefix=${RUSTUP_HOME:-$HOME/.rustup}=/rustup"

echo "Building ${#packages[@]} packages $version for $(dpkg --print-architecture)"
rm -rf "$target/debian"
cargo build --profile dist --locked "${packages[@]/#/--package=}"
for package in "${packages[@]}"; do
    cargo deb --profile dist --no-build --locked --package "$package" --deb-version "$version"
done
ls -l "$target/debian"
