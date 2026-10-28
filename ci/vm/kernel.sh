#!/bin/bash
set -euo pipefail

version=6.12.114
sha256=7a8589912bd7a8adb07fed074283652cf482a35facef8954554654f0154b4af3

here=$(dirname "$(realpath "$0")")
out=$(realpath -m "${1:?usage: kernel.sh <output-dir>}")
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

curl -fsSL -o "$work/linux.tar.xz" \
    "https://cdn.kernel.org/pub/linux/kernel/v6.x/linux-$version.tar.xz"
echo "$sha256  $work/linux.tar.xz" | sha256sum --check --quiet
tar -xJf "$work/linux.tar.xz" -C "$work"
cd "$work/linux-$version"

make -s ARCH=arm64 defconfig
platforms=$(sed -n 's/^config \(ARCH_[A-Z0-9_]*\)$/\1/p' arch/arm64/Kconfig.platforms)
scripts/config --disable MODULES $(printf -- '--disable %s ' $platforms)
scripts/kconfig/merge_config.sh -m .config "$here/kernel.config" > /dev/null
make -s ARCH=arm64 olddefconfig

missing=$(grep -vxFf .config "$here/kernel.config" || true)
if [ -n "$missing" ]; then
    echo "Options the kernel configuration did not take:" >&2
    echo "$missing" >&2
    exit 1
fi

make -s ARCH=arm64 -j"$(nproc)" Image
mkdir -p "$out"
cp arch/arm64/boot/Image "$out/Image"
cp .config "$out/config"
