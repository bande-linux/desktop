#!/bin/bash
# dev-install.sh <package or program>...: installs over the installed packages what each one
# lists in debian/<package>.install (a program names the package that ships it), built from this
# tree with the release profile in the workspace's target/ and the debian/rules targets that
# make debian/build, so it matches the package. Only changed files are copied. Conffiles under
# /etc and what only dpkg handles (PAM files, maintainer scripts, alternatives, the initramfs
# hook, man pages) are skipped and listed. Then it reloads systemd, D-Bus and udev as needed,
# restarts the active units that run a changed program or unit and says when a new login or
# boot is needed. The packages must be installed first; reinstalling one puts its own copy back.
set -euo pipefail
shopt -s nullglob
cd "$(dirname "$0")/.."
die() { echo "dev-install.sh: $*" >&2; exit 1; }
[ $# -gt 0 ] || { echo "usage: dev-install.sh <package or program>..." >&2; exit 2; }
has() { case " $1 " in *" $2 "*) return 0 ;; esac; return 1; }

packages=()
for name; do
    if [ -e "debian/$name.install" ]; then
        package=$name
    else
        owner=$(grep -l "^debian/build/bin/$name " debian/*.install || true)
        [ -n "$owner" ] || die "no package is or installs $name"
        package=$(basename "$owner" .install)
    fi
    [ "$(dpkg-query -W -f='${db:Status-Status}' "$package" 2>/dev/null)" = installed ] ||
        die "$package is not installed; install its package first"
    has "${packages[*]}" "$package" || packages+=("$package")
done

multiarch=$(dpkg-architecture -qDEB_HOST_MULTIARCH)
lines=()
for package in "${packages[@]}"; do
    while read -r source destination; do
        [ -n "$source" ] || continue
        line="$source $destination"
        lines+=("${line//\$\{DEB_HOST_MULTIARCH\}/$multiarch}")
    done < "debian/$package.install"
done

generated=() bins=()
declare -A program_path
for line in "${lines[@]}"; do
    source=${line% *} destination=${line#* }
    case $source in debian/build/*) ;; *) continue ;; esac
    part=${source#debian/build/}
    part=${part%%/*}
    has "${generated[*]}" "$part" || generated+=("$part")
    if [ "$part" = bin ]; then
        bins+=(--bin "${source##*/}")
        program_path[${source##*/}]=/${destination%/}/${source##*/}
    fi
done
! has "${generated[*]}" l10n || bins+=(--bin bande-l10n-tool)
! has "${generated[*]}" cursors || bins+=(--bin bande-cursor-theme)
! has "${generated[*]}" plymouth || bins+=(--bin bande-plymouth-theme)
[ ${#bins[@]} = 0 ] || cargo build --release --locked "${bins[@]}"
! has "${generated[*]}" security || cargo build --release --locked --package libpam-bande-secrets --lib
rm -rf debian/build
[ ${#generated[@]} = 0 ] || make -s -f debian/rules CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}" OUT="${CARGO_TARGET_DIR:-$PWD/target}/release" "${generated[@]}"

changed=() skipped=() unchanged=0
skip() { skipped+=("$1: $2"); }
place() {
    local source=$1 target=$2 mode
    case $target in /etc/*) skip "$target" "a conffile, which only the package installs"; return ;; esac
    if [ -L "$source" ]; then
        if [ -L "$target" ] && [ "$(readlink "$target")" = "$(readlink "$source")" ]; then
            unchanged=$((unchanged + 1))
            return
        fi
        sudo mkdir -p "${target%/*}"
        sudo ln -sfnT "$(readlink "$source")" "$target"
    else
        if cmp -s "$source" "$target"; then
            unchanged=$((unchanged + 1))
            return
        fi
        if [ -e "$target" ]; then
            mode=$(stat -c %a "$target")
        elif [ -x "$source" ]; then
            mode=755
        else
            mode=644
        fi
        sudo install -D -m "$mode" "$source" "$target"
    fi
    changed+=("$target")
}

for line in "${lines[@]}"; do
    pattern=${line% *} destination=/${line#* }
    destination=${destination%/}
    # shellcheck disable=SC2206 # the sources are globs, as for dh_install
    sources=($pattern)
    [ ${#sources[@]} -gt 0 ] || die "$pattern is missing"
    for source in "${sources[@]}"; do
        if [ -d "$source" ] && [ ! -L "$source" ]; then
            while IFS= read -r -d '' file; do
                place "$file" "$destination/${file#"${source%/*}"/}"
            done < <(find "$source" \( -type f -o -type l \) -print0)
        else
            place "$source" "$destination/${source##*/}"
        fi
    done
done
for package in "${packages[@]}"; do
    for file in debian/"$package".*; do
        case ${file##*.} in
            install | lintian-overrides) ;;
            tmpfiles) place "$file" "/usr/lib/tmpfiles.d/$package.conf" ;;
            pam) skip "/etc/pam.d/$package" "a conffile, which only the package installs" ;;
            *) skip "$file" "only the package build and dpkg handle it" ;;
        esac
    done
    ! dpkg -L "$package" | grep -q '^/usr/share/man/man' ||
        skip "the man pages of $package" "debian/manpages.py writes them in the package build"
done
for target in "${changed[@]}"; do echo "installed $target"; done

user_units=() system_units=() themes=() notes=()
system_reload=0 user_reload=0 system_bus=0 session_bus=0 udev=0 desktop=0 login=0
units_running() {
    grep -l "^ExecStart=[-@+!]*$2\( \|$\)" "$1"/*.service 2>/dev/null | xargs -r -n1 basename || true
}
for target in "${changed[@]}"; do
    program=
    case $target in
        /usr/lib/systemd/system/*.d/*) system_reload=1 notes+=("$target applies when its units next start") ;;
        /usr/lib/systemd/user/*.d/*) user_reload=1 notes+=("$target applies when its units next start") ;;
        /usr/lib/systemd/system/*) system_reload=1 system_units+=("${target##*/}") ;;
        /usr/lib/systemd/user/*) user_reload=1 user_units+=("${target##*/}") ;;
        /usr/share/dbus-1/system.d/* | /usr/share/dbus-1/system-services/*) system_bus=1 ;;
        /usr/share/dbus-1/services/*) session_bus=1 ;;
        /usr/lib/udev/rules.d/*) udev=1 ;;
        /usr/lib/tmpfiles.d/*) sudo systemd-tmpfiles --create "$target" ;;
        /usr/lib/modules-load.d/*) notes+=("$target loads its modules from the next boot") ;;
        /usr/share/applications/*) desktop=1 ;;
        /usr/share/icons/*/*)
            theme=${target#/usr/share/icons/}
            theme=/usr/share/icons/${theme%%/*}
            has "${themes[*]}" "$theme" || themes+=("$theme") ;;
        /usr/share/bande/l10n/*/*)
            program=${target#/usr/share/bande/l10n/}
            program=${program%%/*} ;;
        /usr/bin/* | /usr/libexec/*) program=${target##*/} ;;
    esac
    case $target in
        */user@.service.d/* | */security/* | /usr/share/wayland-sessions/* | /usr/share/bande-compositor/* | \
        /usr/bin/bande-session | /usr/bin/bande-compositor | /usr/bin/bande-shell | /usr/bin/bande-lock | \
        /usr/bin/bande-greeter | /usr/lib/systemd/user/bande-compositor.service | /usr/lib/systemd/user/bande-session.target)
            login=1 ;;
    esac
    [ -n "$program" ] && [ -n "${program_path[$program]:-}" ] || continue
    mapfile -t -O "${#user_units[@]}" user_units < <(units_running /usr/lib/systemd/user "${program_path[$program]}")
    mapfile -t -O "${#system_units[@]}" system_units < <(units_running /usr/lib/systemd/system "${program_path[$program]}")
done

[ $system_reload = 0 ] || sudo systemctl daemon-reload
[ $user_reload = 0 ] || systemctl --user daemon-reload
[ $system_bus = 0 ] || { sudo systemctl reload dbus.service; echo "reloaded the system bus"; }
[ $session_bus = 0 ] || { systemctl --user reload dbus.service; echo "reloaded the session bus"; }
[ $udev = 0 ] || { sudo udevadm control --reload; notes+=("the new udev rules apply to devices added from now on"); }
[ $desktop = 0 ] || sudo update-desktop-database -q /usr/share/applications
[ ${#themes[@]} = 0 ] || [ ! -x /usr/sbin/update-icon-caches ] || sudo /usr/sbin/update-icon-caches "${themes[@]}"

no_start=$(sed -n 's/.*--no-start //p' debian/rules | tr '\n' ' ')
restarted=()
restart() {
    local unit=$2
    local systemctl=(systemctl) scope=system
    [ "$1" = system ] || systemctl=(systemctl --user) scope=user
    case $unit in *@.* | *.target) return ;; esac
    if has "${restarted[*]}" "$scope:$unit" || ! "${systemctl[@]}" is-active --quiet "$unit"; then
        return
    fi
    if [ "$("${systemctl[@]}" show -P Type "$unit")" = oneshot ] || { [ $scope = system ] && has "$no_start" "$unit"; }; then
        notes+=("$unit uses the change from its next start")
        return
    fi
    if [ $scope = system ]; then sudo systemctl restart "$unit"; else systemctl --user restart "$unit"; fi
    restarted+=("$scope:$unit")
    echo "restarted $unit"
}
for unit in "${user_units[@]}"; do
    [ "$unit" = bande-compositor.service ] || restart user "$unit"
done
for unit in "${system_units[@]}"; do
    if [ "$unit" = bande-greeter.service ]; then
        notes+=("the login window runs the new greeter from the next boot")
    else
        restart system "$unit"
    fi
done

for entry in "${skipped[@]}"; do echo "skipped $entry"; done
for note in "${notes[@]}"; do echo "note: $note"; done
echo "${#changed[@]} files installed, $unchanged unchanged, ${#skipped[@]} skipped (${packages[*]})"
[ $login = 0 ] || echo "log out and in to use the new session"
