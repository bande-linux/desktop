#!/bin/bash
set -euo pipefail

binaries=$1 out=$2 version=$3
scratch=$(mktemp -d)
trap 'rm -rf "$scratch"' EXIT
root=$scratch/root
mkdir -p "$root/DEBIAN" "$out"

while IFS= read -r binary; do
    readelf --section-headers "$binary" | grep -q '\.debug_' || continue
    id=$(readelf --notes "$binary" | sed -n 's/.*Build ID: //p')
    [ -n "$id" ] || { echo "$binary has no build id" >&2; exit 1; }
    debug=$root/usr/lib/debug/.build-id/${id:0:2}/${id:2}.debug
    mkdir -p "$(dirname "$debug")"
    objcopy --only-keep-debug --compress-debug-sections=zlib "$binary" "$debug"
    nm --dynamic --format=posix --defined-only "$binary" | cut -d' ' -f1 | sort -u > "$scratch/exported"
    nm --format=posix --defined-only "$binary" | awk '$2 == "T" || $2 == "t" { print $1 }' | sort -u > "$scratch/functions"
    comm -13 "$scratch/exported" "$scratch/functions" > "$scratch/kept"
    objcopy --only-keep-debug "$binary" "$scratch/whole"
    objcopy --strip-debug --remove-section=.comment --keep-symbols="$scratch/kept" "$scratch/whole" "$scratch/names"
    xz --force --threads=0 "$scratch/names"
    strip --strip-all "$binary"
    objcopy --add-section .gnu_debugdata="$scratch/names.xz" "$binary"
done < <(find "$binaries" -maxdepth 1 -type f \( -executable -o -name '*.so' \))

cat > "$root/DEBIAN/control" <<CONTROL
Package: bande-debug-symbols
Version: $version
Architecture: $(dpkg --print-architecture)
Section: debug
Priority: optional
Maintainer: Mikael Hug <mikaelhug@gmail.com>
Description: Debug symbols of the Bande desktop
 Files and line numbers for the stack traces of the Bande desktop, found by
 build id. coredumpctl debug and other debuggers read them.
CONTROL
dpkg-deb --root-owner-group --build "$root" "$out/bande-debug-symbols_${version}_$(dpkg --print-architecture).deb"
