#!/bin/bash
# build.sh: run as root in debian:<suite>, the Debian release the package is for, sharing <work dir> between steps.
#   build.sh fetch <debian version> <work dir>    Debian's firefox-esr source through apt, checked
#                                                 against the signed Release
#   (then scripts/prepare.sh <work dir>/firefox-esr-* <version>)
#   build.sh compile <version> <work dir> <out>   Debian's own build into <out>
set -euo pipefail
export DEBIAN_FRONTEND=noninteractive

case ${1:-} in
fetch)
    debver=$2 work=$(realpath -m "$3")
    keyring=
    [ ! -f /etc/apt/sources.list.d/debian.sources ] ||
        keyring=$(sed -n 's/^Signed-By: //p' /etc/apt/sources.list.d/debian.sources | head -1)
    for key in /usr/share/keyrings/debian-archive-keyring.pgp /usr/share/keyrings/debian-archive-keyring.gpg; do
        [ -n "$keyring" ] || [ ! -f "$key" ] || keyring=$key
    done
    suite=$(sed -n 's/^VERSION_CODENAME=//p' /etc/os-release | tr -d '"')
    printf 'Types: deb-src\nURIs: http://deb.debian.org/debian\nSuites: %s unstable\nComponents: main\nSigned-By: %s\n\nTypes: deb-src\nURIs: http://security.debian.org/debian-security\nSuites: %s-security\nComponents: main\nSigned-By: %s\n' "$suite" "$keyring" "$suite" "$keyring" \
        > /etc/apt/sources.list.d/debian-src.sources
    apt-get update -qq
    apt-get install -y -qq --no-install-recommends devscripts quilt python3 debhelper dpkg-dev \
        fakeroot ca-certificates librsvg2-bin iso-codes > /dev/null
    mkdir -p "$work"
    cd "$work"
    apt-get source -qq "firefox-esr=$debver" > /dev/null
    rm -f "$work"/firefox-esr_*.orig* "$work"/firefox-esr_*.debian.tar.xz
    ls -d "$work"/firefox-esr-*/
    ;;
compile)
    version=$2 work=$(realpath "$3") out=$(realpath -m "$4")
    src=$(ls -d "$work"/firefox-esr-*/ | head -1)
    cd "$src"
    apt-get build-dep -y -qq . > /dev/null
    jobs=${BROWSER_JOBS:-$(nproc)}
    start_note="jobs $jobs, Rust ${BROWSER_RUST_LTO:--Clto=thin}"
    echo "build: $start_note"
    start=$SECONDS
    DEB_BUILD_OPTIONS="parallel=$jobs nocheck" DEBIAN_RUST_LTO=${BROWSER_RUST_LTO:--Clto=thin} dpkg-buildpackage -b -uc -us -nc
    echo "build: $(( (SECONDS - start) / 60 )) min"
    mkdir -p "$out"
    cp "$work"/bande-browser_"$version"_*.deb "$out/"
    for lang in $(tr '[:upper:]' '[:lower:]' < debian/bande-languages); do
        cp "$work/bande-browser-l10n-${lang}_${version}_all.deb" "$out/"
    done
    ls -l "$out"
    ;;
*)
    echo "usage: build.sh fetch|compile ..." >&2
    exit 2
    ;;
esac
