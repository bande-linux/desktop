#!/bin/bash
# vendor.sh <out dir> <upstream version>: the component tarball
# bande-desktop_<version>.orig-vendor.tar.xz with every crate of Cargo.lock (cargo vendor
# --versioned-dirs), which the pkg.bande.vendor build profile builds from. A crate that no Linux
# build on amd64 or arm64 uses keeps only its Cargo.toml, which cargo still reads to resolve, and
# an empty src/lib.rs. The Files-Excluded of ci/copyright and cargo package's Cargo.toml.orig,
# which dh_clean deletes, are left out.
set -euo pipefail
cd "$(dirname "$0")/../.."
out=$(realpath "${1:?usage: vendor.sh <out dir> <upstream version>}")
version=${2:?usage: vendor.sh <out dir> <upstream version>}
epoch=${SOURCE_DATE_EPOCH:-$(git log -1 --format=%ct)}
scratch=$(mktemp -d)
trap 'rm -rf "$scratch"' EXIT

cargo vendor --locked --versioned-dirs --quiet "$scratch/vendor" > /dev/null
for triple in x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu; do
    cargo metadata --locked --offline --format-version 1 --filter-platform "$triple" |
        jq -r '.resolve.nodes[].id as $id | .packages[] | select(.id == $id and .source != null) | "\(.name)-\(.version)"'
done | sort -u > "$scratch/linux"
excluded=$(awk '/^Files-Excluded:/ { sub(/^Files-Excluded:/, ""); listing = 1 } listing && /^[^ ]/ && !/^Files-Excluded:/ { exit }
    listing { print }' ci/copyright)
# shellcheck disable=SC2086 # the patterns are globs
(cd "$scratch" && for pattern in $excluded; do rm -rf -- $pattern; done)
find "$scratch/vendor" -name Cargo.toml.orig -delete
for crate in "$scratch"/vendor/*; do
    if ! grep -qxF "$(basename "$crate")" "$scratch/linux"; then
        find "$crate" -mindepth 1 -maxdepth 1 ! -name Cargo.toml ! -name .cargo-checksum.json -exec rm -rf {} +
        mkdir "$crate/src"
        : > "$crate/src/lib.rs"
    fi
    (cd "$crate" && find . -type f ! -name .cargo-checksum.json -printf '%P\n' | LC_ALL=C sort | xargs -r -d '\n' sha256sum) |
        jq -R -s 'split("\n") | map(select(length > 0) | capture("^(?<sum>[0-9a-f]+)  (?<file>.*)$") | {(.file): .sum}) | add // {}' \
            > "$scratch/files"
    jq -c --slurpfile files "$scratch/files" '.files = $files[0]' "$crate/.cargo-checksum.json" > "$scratch/checksum"
    mv "$scratch/checksum" "$crate/.cargo-checksum.json"
done
mkdir -p "$out"
tar --create --directory "$scratch" --sort=name --mtime="@$epoch" --owner=0 --group=0 --numeric-owner \
    --mode=u+rw,go=rX,go-w --pax-option=exthdr.name=%d/PaxHeaders/%f,delete=atime,delete=ctime vendor |
    xz --threads=1 -6 > "$out/bande-desktop_$version.orig-vendor.tar.xz"
