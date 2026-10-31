#!/bin/bash
# Points the workspace's target directory at one that persists on this runner between jobs, so
# a job compiles only what changed since the runner's last build. Every job on the runner shares
# it: cargo keeps each profile and feature set apart and locks the directory while it builds.
# It is emptied when the toolchain changes, when it outgrows its budget or when the disk runs low.
set -euo pipefail

root=${UNBOUND_TARGET_ROOT:-/cache/target}/${RUNNER_NAME:-local}
budget_gib=${UNBOUND_TARGET_BUDGET_GIB:-60}
min_free_gib=${UNBOUND_TARGET_MIN_FREE_GIB:-20}
toolchain=$(sed -n 's/^channel = "\(.*\)"$/\1/p' rust-toolchain.toml)
dir="$root/$toolchain"

mkdir -p "$root"
find "$root" -mindepth 1 -maxdepth 1 ! -name "$toolchain" -exec rm -rf {} +
mkdir -p "$dir"

used_gib=$(( $(du -s --block-size=1G "$dir" | cut -f1) ))
free_gib=$(( $(df --output=avail --block-size=1G "$root" | tail -n 1) ))
echo "Target directory $dir: ${used_gib} GiB used, ${free_gib} GiB free"
if [ "$used_gib" -gt "$budget_gib" ] || [ "$free_gib" -lt "$min_free_gib" ]; then
    echo "Emptying it"
    find "$dir" -mindepth 1 -maxdepth 1 -exec rm -rf {} +
fi
rm -rf "$dir/debian" "$dir/tmp"

rm -rf target
ln -s "$dir" target
