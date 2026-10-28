#!/bin/bash
set -euo pipefail

here=$(dirname "$(realpath "$0")")
out=$(realpath -m "${1:?usage: images.sh <output-dir>}")
cache=${VM_IMAGE_CACHE:-/cache/unbound-vm}

recipe_hash() {
    (cd "$here" && sha256sum "$@") | sha256sum | cut -c1-16
}

cached() {
    local name=$1 builder=$2
    shift 2
    local dir="$cache/$name-$(recipe_hash "$@")"
    if [ ! -d "$dir" ]; then
        local staging="$dir.$$"
        rm -rf "$staging"
        "$here/$builder" "$staging" >&2
        mv "$staging" "$dir"
    fi
    echo "$dir"
}

mkdir -p "$cache" "$out"
kernel=$(cached kernel kernel.sh kernel.sh kernel.config)
rootfs=$(cached rootfs rootfs.sh rootfs.sh Dockerfile init)
ln -sf "$kernel/Image" "$out/Image"
ln -sf "$rootfs/rootfs.img" "$out/rootfs.img"
