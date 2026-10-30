#!/bin/bash
set -euo pipefail

here=$(dirname "$(realpath "$0")")
out=$(realpath -m "${1:?usage: images.sh <output-dir>}")
cache=${VM_IMAGE_CACHE:-/cache/unbound-vm}

recipe_hash() {
    (cd "$here" && sha256sum "$@") | sha256sum | cut -c1-16
}

# The cache is shared by all runners. A recipe is built by one of them while the others wait for
# it on the recipe's lock, and a recipe's modification time records when a job last used it.
cached() {
    local name=$1 builder=$2 lock
    shift 2
    local dir="$cache/$name-$(recipe_hash "$@")"
    exec {lock}>"$dir.lock"
    flock "$lock"
    if [ ! -d "$dir" ]; then
        local staging="$dir.$$"
        rm -rf "$staging"
        "$here/$builder" "$staging" >&2
        mv "$staging" "$dir"
    fi
    touch "$dir"
    echo "$dir"
}

prune() {
    local old
    find "$cache" -mindepth 1 -maxdepth 1 -type d -mtime +7 -print0 |
        while IFS= read -r -d '' old; do
            (flock -n 9 && [ -n "$(find "$old" -maxdepth 0 -mtime +7)" ] && rm -rf "$old" "$old.lock") \
                9>"$old.lock" || true
        done
}

mkdir -p "$cache" "$out"
prune
kernel=$(cached kernel kernel.sh kernel.sh kernel.config)
rootfs=$(cached rootfs rootfs.sh rootfs.sh Dockerfile init login.service)
ln -sf "$kernel/Image" "$out/Image"
ln -sf "$rootfs/rootfs.img" "$out/rootfs.img"
