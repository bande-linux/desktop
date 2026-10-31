#!/bin/bash
set -euo pipefail

usage="usage: initramfs.sh <rootfs.img> <guest-test-binary> <out.cpio.xz>"
rootfs=$(realpath "${1:?$usage}")
binary=$(realpath "${2:?$usage}")
out=$(realpath -m "${3:?$usage}")
here=$(dirname "$(realpath "$0")")
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

full=$work/full
root=$work/root
mkdir "$full" "$root"
debugfs -R "rdump / $full" "$rootfs" 2> /dev/null
mkdir -p "$full/bande"
strip -o "$full/bande/guest" "$binary"

cd "$full"
lib=usr/lib/aarch64-linux-gnu
programs=(bande/guest usr/bin/dash usr/bin/base64 usr/bin/sleep usr/bin/ls usr/bin/mount usr/sbin/seatd)
modules=("$lib"/libvulkan.so.1 "$lib"/libvulkan_lvp.so)
data=(usr/share/vulkan/icd.d usr/share/X11/xkb usr/share/drirc.d etc/passwd etc/group)

needed() {
    chroot "$full" /usr/lib/ld-linux-aarch64.so.1 --list "/$1" | awk '/=>/ {print $3} !/=>/ {print $1}'
}

files=$(
    for item in "${programs[@]}" "${modules[@]}"; do
        echo "$item"
        needed "$item" | sed 's|^/||'
    done
    find "${data[@]}" -print
)
printf '%s\n' $files usr/lib/ld-linux-aarch64.so.1 | grep -v '^linux-vdso' \
    | sed 's|^lib/|usr/lib/|; s|^bin/|usr/bin/|; s|^sbin/|usr/sbin/|' | sort -u \
    | cpio --quiet -pdmL "$root"
ln -s usr/bin "$root/bin"
ln -s usr/sbin "$root/sbin"
ln -s usr/lib "$root/lib"
ln -s dash "$root/usr/bin/sh"
mkdir -p "$root"/{dev,proc,sys,run,tmp,root}
cp "$here/init" "$root/init"
cd "$root"
find . -print0 | cpio --null -o -H newc --quiet | xz -T0 --check=crc32 -9 > "$out"
