#!/bin/bash
set -uo pipefail
cd "$(dirname "$0")/../.."

hours=4
fuzz_seconds=600
out=soak-out
while [ $# -gt 0 ]; do
    case $1 in
        --hours) hours=$2; shift ;;
        --fuzz-seconds) fuzz_seconds=$2; shift ;;
        --out) out=$2; shift ;;
        *) echo "usage: ci/soak/run.sh [--hours H] [--fuzz-seconds S] [--out DIR]" >&2; exit 2 ;;
    esac
    shift
done
if dpkg-query -W -f '${db:Status-Status}' bande-lock 2>/dev/null | grep -q '^installed$'; then
    echo "ci/soak/run.sh is for build machines: this one has the desktop installed" >&2
    exit 2
fi

needed=(systemd-coredump libdw1t64 libpam-runtime mesa-vulkan-drivers dbus-daemon)
missing=()
for package in "${needed[@]}"; do
    [ "$(dpkg-query -W -f '${db:Status-Status}' "$package" 2>/dev/null)" = installed ] || missing+=("$package")
done
[ ${#missing[@]} -eq 0 ] || sudo -n apt-get install -y -q --no-install-recommends "${missing[@]}" || exit 2
printf 'auth required pam_permit.so\naccount required pam_permit.so\n' | sudo -n tee /etc/pam.d/bande-lock > /dev/null || exit 2

sudo -n prlimit --core=unlimited:unlimited --pid $$ || exit 2

mkdir -p "$out"
out=$(realpath "$out")
export VK_DRIVER_FILES=/usr/share/vulkan/icd.d/lvp_icd.json
export CARGO_PROFILE_RELEASE_DEBUG=line-tables-only
started=$(date '+%Y-%m-%d %H:%M:%S')
cargo build --release --locked -p bande-bench --bin bande-soak || exit 2
cargo build --release --locked -p bande-shell -p bande-lock -p bande-files -p bande-settings \
    -p bande-notify -p bande-config || exit 2
target=${CARGO_TARGET_DIR:-target}/release

timeout --kill-after=60 --signal=ABRT "$(awk "BEGIN { print int($hours * 3600 + 1800) }")" \
    "$target/bande-soak" --hours "$hours" --out "$out" > "$out/soak.log" 2>&1
soak=$?

{
    echo "== crashes on this machine since $started that the soak did not cause"
    unexpected=0
    while read -r pid; do
        grep -qx "$pid" "$out/expected-crashes" 2>/dev/null && continue
        unexpected=$((unexpected + 1))
        coredumpctl info --no-pager -q "$pid"
    done < <(coredumpctl list --no-pager --no-legend --since "$started" -q 2>/dev/null | awk '{ print $5 }')
    echo "unexpected crashes: $unexpected"
} > "$out/crashes.txt" 2>&1
if [ "$soak" = 0 ] && ! grep -q '^unexpected crashes: 0$' "$out/crashes.txt"; then
    soak=1
fi

fuzz=0
if [ "$fuzz_seconds" -gt 0 ]; then
    command -v cargo-fuzz > /dev/null || cargo install cargo-fuzz --locked || exit 2
    corpus=$HOME/.cache/bande-fuzz/desktop
    cp Cargo.lock fuzz/Cargo.lock
    targets=(wayland_wire file_preview text_formats config_files privileged_lines)
    each=$((fuzz_seconds / ${#targets[@]}))
    for name in "${targets[@]}"; do
        mkdir -p "$corpus/$name" "$out/fuzz/$name"
        (cd fuzz && cargo fuzz run --sanitizer none "$name" "$corpus/$name" -- \
            -max_total_time="$each" -rss_limit_mb=4096 -timeout=30 \
            -artifact_prefix="$out/fuzz/$name/" -print_final_stats=1) > "$out/fuzz/$name.log" 2>&1 || fuzz=1
    done
fi

{
    date '+%Y-%m-%d %H:%M:%S'
    git log -1 --format='%h %s' 2>/dev/null
    [ "$soak" = 0 ] && echo "soak: pass" || echo "soak: FAIL ($soak)"
    [ "$fuzz" = 0 ] && echo "fuzz: pass" || echo "fuzz: FAIL, see fuzz/"
    cat "$out/summary.txt" 2>/dev/null || echo "the soak ended without a summary, see soak.log"
    tail -1 "$out/crashes.txt"
} > "$out/result.txt"
cat "$out/result.txt"
[ "$soak" = 0 ] && [ "$fuzz" = 0 ]
