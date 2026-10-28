#!/bin/bash
set -euo pipefail

images=$(realpath "${1:?usage: run.sh <images-dir> <share-dir>}")
share=$(realpath "${2:?usage: run.sh <images-dir> <share-dir>}")
test -f "$share/run"
rm -f "$share/status"

status=0
timeout --foreground "${GUEST_TIMEOUT:-600}" qemu-system-aarch64 \
    -machine virt,gic-version=max,its=off -accel kvm -cpu host -smp 4 -m 4096 \
    -kernel "$images/Image" \
    -append "console=ttyAMA0 root=/dev/vda rw rootfstype=ext4 init=/sbin/unbound-guest-init panic=-1 loglevel=4 vkms.enable_overlay=1" \
    -drive if=none,id=root,format=raw,file="$images/rootfs.img",snapshot=on \
    -device virtio-blk-pci,drive=root \
    -virtfs local,path="$share",mount_tag=share,security_model=none,id=share \
    -device virtio-gpu-pci \
    -display none -monitor none -serial stdio -no-reboot -nodefaults \
    < /dev/null || status=$?

if [ "$status" -ne 0 ]; then
    echo "QEMU exited with status $status" >&2
    exit 1
fi
if [ ! -f "$share/status" ]; then
    echo "The guest stopped without reporting a status" >&2
    exit 1
fi
exit "$(cat "$share/status")"
