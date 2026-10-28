#!/bin/bash
set -euo pipefail

here=$(dirname "$(realpath "$0")")
out=$(realpath -m "${1:?usage: rootfs.sh <output-dir>}")
work=$(mktemp -d)
image=
container=
cleanup() {
    [ -n "$container" ] && docker rm -f "$container" > /dev/null
    [ -n "$image" ] && docker rmi -f "$image" > /dev/null
    rm -rf "$work"
}
trap cleanup EXIT

image=$(docker build --quiet --platform linux/arm64 "$here")
container=$(docker create --platform linux/arm64 "$image")
mkdir "$work/root"
docker export "$container" | tar -x -C "$work/root" --numeric-owner
rm -f "$work/root/.dockerenv"

size_mib=$(( $(du -sm "$work/root" | cut -f1) * 3 / 2 + 256 ))
mkdir -p "$out"
rm -f "$out/rootfs.img"
mkfs.ext4 -q -L root -d "$work/root" "$out/rootfs.img" "${size_mib}M"
