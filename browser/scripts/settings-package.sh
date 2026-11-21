#!/bin/bash
# settings-package.sh <version> <out>: as root in debian:<suite>, builds bande-firefox-esr-settings
# from firefox-esr-bande/ at <version>, the browser's, into <out>.
set -euo pipefail
version=$1 out=$(realpath -m "$2")
repo=$(dirname "$(dirname "$(realpath "$0")")")
export DEBIAN_FRONTEND=noninteractive
apt-get install -y -qq --no-install-recommends debhelper dpkg-dev > /dev/null
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
cp -r "$repo/firefox-esr-bande" "$work/"
cd "$work/firefox-esr-bande"
suite=$(sed -n 's/^VERSION_CODENAME=//p' /etc/os-release)
{
    echo "bande-firefox-esr-settings ($version) $suite; urgency=medium"
    echo
    echo "  * The settings of Bande's Browser $version."
    echo
    echo " -- $(sed -n 's/^Maintainer: //p' debian/control)  $(date -R)"
    echo
    cat debian/changelog
} > debian/changelog.new
mv debian/changelog.new debian/changelog
dpkg-buildpackage -b -uc -us
mkdir -p "$out"
cp "$work/bande-firefox-esr-settings_${version}_all.deb" "$out/"
