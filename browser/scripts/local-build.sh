#!/bin/bash
# local-build.sh <debian version> <dir> [step...]: the build on any Debian machine, without root or
# containers: a Debian testing chroot made with mmdebstrap's unshare mode. <dir> must be reachable
# by other users (/var/tmp/...). Steps, all but clean by default:
#   chroot   make <dir>/chroot (once)
#   fetch    Debian's source into <dir>/work (emptied first)
#   prepare  branding, prefs and patches (prepare.sh), from this checkout
#   compile  build the package into <dir>/out (emptied first)
#   clean    remove <dir>
set -euo pipefail
debver=$1 dir=$(realpath -m "$2"); shift 2
repo=$(dirname "$(dirname "$(realpath "$0")")")
version=${BROWSER_VERSION:-$("$repo/scripts/version.sh" "$debver")}
steps=("$@"); [ ${#steps[@]} -gt 0 ] || steps=(chroot fetch prepare compile)

as_root() { unshare --map-auto --map-root-user --mount --pid --fork --kill-child "$@"; }
in_chroot() {
    mkdir -p "$dir/repo" "$dir/work" "$dir/out"
    rsync -a --delete --exclude .git "$repo/" "$dir/repo/"
    as_root bash -c '
        d=$0; c=$d/chroot
        mount -t proc proc "$c/proc"
        mount --rbind /dev "$c/dev"
        for m in repo work out; do mkdir -p "$c/$m"; mount --bind "$d/$m" "$c/$m"; done
        cp /etc/resolv.conf "$c/etc/resolv.conf"
        exec /usr/sbin/chroot "$c" env -i PATH=/usr/sbin:/usr/bin:/sbin:/bin HOME=/root LANG=C.UTF-8 BROWSER_JOBS="${BROWSER_JOBS:-}" BROWSER_RUST_LTO="${BROWSER_RUST_LTO:-}" "$@"
    ' "$dir" "$@"
}

for step in "${steps[@]}"; do
    echo "local-build: $step ($version)"
    case $step in
    chroot)
        [ -d "$dir/chroot/etc" ] && continue
        mkdir -p "$dir"
        chmod 755 "$dir"
        mmdebstrap --mode=unshare --variant=buildd --include=ca-certificates testing "$dir/chroot"
        ;;
    fetch)
        as_root rm -rf "$dir/work"
        in_chroot /repo/scripts/build.sh fetch "$debver" /work
        ;;
    prepare) in_chroot sh -c '/repo/scripts/prepare.sh /work/firefox-esr-*/ "$0"' "$version" ;;
    compile)
        as_root rm -rf "$dir/out"
        in_chroot /repo/scripts/build.sh compile "$version" /work /out
        ;;
    clean) as_root rm -rf "$dir" ;;
    *) echo "local-build: unknown step $step" >&2; exit 2 ;;
    esac
done
