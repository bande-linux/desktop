#!/bin/bash
set -euo pipefail

list=$1 staged=$2 out=$3 version=$4
scratch=$(mktemp -d)
trap 'rm -rf "$scratch"' EXIT
root=$scratch/root
rm -rf "$staged"
mkdir -p "$root/DEBIAN" "$out" "$staged"

while IFS= read -r binary; do
    [ "$(od -An -tx1 -N4 "$binary" | tr -d ' ')" = 7f454c46 ] || continue
    copy=$staged/$(basename "$binary")
    cp --preserve=mode,timestamps "$binary" "$copy"
    readelf --wide --section-headers "$copy" > "$scratch/sections"
    grep -q '\.debug_info' "$scratch/sections" || { echo "$binary has no debug info" >&2; exit 1; }
    readelf --notes "$copy" > "$scratch/notes"
    id=$(sed -n 's/.*Build ID: //p' "$scratch/notes")
    [ -n "$id" ] || { echo "$binary has no build id" >&2; exit 1; }
    debug=$root/usr/lib/debug/.build-id/${id:0:2}/${id:2}.debug
    mkdir -p "$(dirname "$debug")"
    objcopy --only-keep-debug --compress-debug-sections=zlib "$copy" "$debug"
    nm --dynamic --format=posix --defined-only "$copy" | cut -d' ' -f1 | sort -u > "$scratch/exported"
    nm --format=posix --defined-only "$copy" | awk '$2 == "T" || $2 == "t" { print $1 }' | sort -u > "$scratch/functions"
    comm -13 "$scratch/exported" "$scratch/functions" > "$scratch/kept"
    objcopy --only-keep-debug "$copy" "$scratch/whole"
    objcopy --strip-debug --remove-section=.comment --keep-symbols="$scratch/kept" "$scratch/whole" "$scratch/names"
    xz --force --threads=0 "$scratch/names"
    strip --strip-all "$copy"
    objcopy --add-section .gnu_debugdata="$scratch/names.xz" "$copy"
    readelf --wide --section-headers "$copy" > "$scratch/sections"
    if grep -qE '\.debug_(info|line|str) ' "$scratch/sections" || ! grep -q '\.gnu_debugdata' "$scratch/sections"; then
        echo "$copy is not stripped to its .gnu_debugdata" >&2
        exit 1
    fi
done < "$list"

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
