#!/bin/bash
set -euo pipefail

[ $# -eq 2 ] || { echo "usage: run.sh <old-debs-dir> <new-debs-dir>" >&2; exit 2; }
old=$(realpath "$1")
new=$(realpath "$2")
here=$(realpath "$(dirname "$0")")
cache=${XDG_CACHE_HOME:-$HOME/.cache}/bande-ci/upgrade-test
mkdir -p "$cache"
scratch=$(mktemp -d -p "$cache")
vm=
cleanup() {
    [ -z "$vm" ] || kill "$vm" 2>/dev/null || true
    rm -rf "$scratch"
}
trap cleanup EXIT
trap 'exit 143' TERM INT HUP

case $(dpkg --print-architecture) in
    arm64)
        image=debian-14-genericcloud-arm64-daily.qcow2
        cp /usr/share/AAVMF/AAVMF_VARS.fd "$scratch/vars.fd"
        qemu=(qemu-system-aarch64 -machine "virt,gic-version=max,its=off" -cpu host
            -drive "if=pflash,format=raw,readonly=on,file=/usr/share/AAVMF/AAVMF_CODE.fd"
            -drive "if=pflash,format=raw,file=$scratch/vars.fd")
        ;;
    amd64)
        image=debian-14-genericcloud-amd64-daily.qcow2
        qemu=(qemu-system-x86_64 -machine q35 -cpu host)
        ;;
    *) echo "run.sh: no image for this architecture" >&2; exit 1 ;;
esac

[ -f "$cache/$image" ] && [ -z "$(find "$cache/$image" -mtime +7)" ] ||
    curl -fsSL --retry 3 -o "$cache/$image" "https://cloud.debian.org/images/cloud/forky/daily/latest/$image"
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

guest=(-i "$scratch/key" -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR -o ConnectTimeout=5 -o ServerAliveInterval=15 -o ServerAliveCountMax=8)

boot() {
    port=$((20000 + RANDOM % 20000))
    qemu-img create -q -f qcow2 -F qcow2 -b "$cache/$image" "$scratch/disk.qcow2" 12G
    "${qemu[@]}" -enable-kvm -smp "${UPGRADE_TEST_CPUS:-4}" -m "${UPGRADE_TEST_MEMORY:-6G}" -nographic \
        -serial "file:${UPGRADE_TEST_SERIAL:-/dev/null}" -monitor none \
        -drive "file=$scratch/disk.qcow2,if=virtio" \
        -drive "file=$scratch/seed.iso,if=virtio,format=raw,readonly=on" \
        -nic "user,model=virtio-net-pci,hostfwd=tcp:127.0.0.1:$port-:22" &
    vm=$!
    for _ in $(seq 36); do
        if timeout 120 ssh "${guest[@]}" -p "$port" root@127.0.0.1 'cloud-init status --wait > /dev/null; true' 2>/dev/null; then
            return 0
        fi
        kill -0 "$vm" || return 1
        sleep 5
    done
    return 1
}

booted=
for _ in 1 2 3; do
    if boot; then
        booted=1
        break
    fi
    echo "run.sh: the VM did not come up, starting another" >&2
    kill "$vm" 2>/dev/null || true
    wait "$vm" 2>/dev/null || true
done
[ -n "$booted" ] || { echo "run.sh: the VM never answered" >&2; exit 1; }

scp "${guest[@]}" -q -P "$port" -r "$old" root@127.0.0.1:/root/old
scp "${guest[@]}" -q -P "$port" -r "$new" root@127.0.0.1:/root/new
scp "${guest[@]}" -q -P "$port" "$here/in-guest.sh" root@127.0.0.1:/root/in-guest.sh
ssh "${guest[@]}" -p "$port" root@127.0.0.1 bash /root/in-guest.sh /root/old /root/new
