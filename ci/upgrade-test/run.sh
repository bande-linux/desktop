#!/bin/bash
set -euo pipefail

[ $# -eq 2 ] || { echo "usage: run.sh <old-debs-dir> <new-debs-dir>" >&2; exit 2; }
old=$(realpath "$1")
new=$(realpath "$2")
here=$(realpath "$(dirname "$0")")
cache=${XDG_CACHE_HOME:-$HOME/.cache}/bande-ci/upgrade-test
scratch=$(mktemp -d)
vm=
cleanup() {
    [ -z "$vm" ] || kill "$vm" 2>/dev/null || true
    rm -rf "$scratch"
}
trap cleanup EXIT

case $(dpkg --print-architecture) in
    arm64)
        image=debian-14-genericcloud-arm64-daily.qcow2
        qemu=(qemu-system-aarch64 -machine virt -cpu host
            -bios /usr/share/qemu-efi-aarch64/QEMU_EFI.fd)
        ;;
    amd64)
        image=debian-14-genericcloud-amd64-daily.qcow2
        qemu=(qemu-system-x86_64 -machine q35 -cpu host)
        ;;
    *) echo "run.sh: no image for this architecture" >&2; exit 1 ;;
esac

mkdir -p "$cache"
[ -f "$cache/$image" ] && [ -z "$(find "$cache/$image" -mtime +7)" ] ||
    curl -fsSL --retry 3 -o "$cache/$image" "https://cloud.debian.org/images/cloud/forky/daily/latest/$image"
qemu-img create -q -f qcow2 -F qcow2 -b "$cache/$image" "$scratch/disk.qcow2" 12G

ssh-keygen -q -t ed25519 -N '' -f "$scratch/key"
mkdir "$scratch/seed"
echo "instance-id: bande-upgrade-test" > "$scratch/seed/meta-data"
cat > "$scratch/seed/user-data" <<DATA
#cloud-config
users:
  - name: root
    ssh_authorized_keys: ["$(cat "$scratch/key.pub")"]
disable_root: false
DATA
genisoimage -quiet -output "$scratch/seed.iso" -volid cidata -joliet -rock "$scratch/seed"

port=$((20000 + RANDOM % 20000))
"${qemu[@]}" -enable-kvm -smp "${UPGRADE_TEST_CPUS:-4}" -m "${UPGRADE_TEST_MEMORY:-6G}" -nographic -serial null -monitor none \
    -drive "file=$scratch/disk.qcow2,if=virtio" \
    -drive "file=$scratch/seed.iso,if=virtio,format=raw,readonly=on" \
    -nic "user,model=virtio-net-pci,hostfwd=tcp:127.0.0.1:$port-:22" &
vm=$!

guest=(-i "$scratch/key" -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR -o ConnectTimeout=5)
ready=
for _ in $(seq 120); do
    if ssh "${guest[@]}" -p "$port" root@127.0.0.1 'cloud-init status --wait > /dev/null; true' 2>/dev/null; then
        ready=1
        break
    fi
    kill -0 "$vm" || { echo "run.sh: the VM exited" >&2; exit 1; }
    sleep 5
done
[ -n "$ready" ] || { echo "run.sh: the VM never answered" >&2; exit 1; }

scp "${guest[@]}" -q -P "$port" -r "$old" root@127.0.0.1:/root/old
scp "${guest[@]}" -q -P "$port" -r "$new" root@127.0.0.1:/root/new
scp "${guest[@]}" -q -P "$port" "$here/in-guest.sh" root@127.0.0.1:/root/in-guest.sh
ssh "${guest[@]}" -p "$port" root@127.0.0.1 bash /root/in-guest.sh /root/old /root/new
