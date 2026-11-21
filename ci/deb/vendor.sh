#!/bin/bash
# vendor.sh <out dir> <upstream version>: the component tarball
# bande-desktop_<version>.orig-vendor.tar.xz with every crate of Cargo.lock (cargo vendor
# --versioned-dirs), which the pkg.bande.vendor build profile builds from. A crate that no Linux
# build on amd64 or arm64 uses keeps only its Cargo.toml, which cargo still reads to resolve, and
# an empty src/lib.rs.
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
for crate in "$scratch"/vendor/*; do
    grep -qxF "$(basename "$crate")" "$scratch/linux" && continue
    find "$crate" -mindepth 1 -maxdepth 1 ! -name Cargo.toml ! -name .cargo-checksum.json -exec rm -rf {} +
    mkdir "$crate/src"
    : > "$crate/src/lib.rs"
    jq -c --arg empty "$(sha256sum < /dev/null | cut -d' ' -f1)" \
        '.files |= (with_entries(select(.key == "Cargo.toml")) + {"src/lib.rs": $empty})' \
        "$crate/.cargo-checksum.json" > "$scratch/checksum"
    mv "$scratch/checksum" "$crate/.cargo-checksum.json"
done
mkdir -p "$out"
tar --create --directory "$scratch" --sort=name --mtime="@$epoch" --owner=0 --group=0 --numeric-owner \
    --mode=u+rw,go=rX,go-w --pax-option=exthdr.name=%d/PaxHeaders/%f,delete=atime,delete=ctime vendor |
    xz --threads=1 -6 > "$out/bande-desktop_$version.orig-vendor.tar.xz"
