#!/bin/bash
# check-package.sh <deb>: as root in a debian:<suite> container with the Bande archive: installs the
# package and proves a fresh profile stays silent for 10 minutes. No release passes without this.
set -euo pipefail
deb=$(realpath "$1")
repo=$(dirname "$(dirname "$(realpath "$0")")")
export DEBIAN_FRONTEND=noninteractive
apt-get update -qq
apt-get install -y -qq --no-install-recommends ca-certificates curl tcpdump iproute2 python3 dbus \
    mesa-vulkan-drivers > /dev/null
suite=$(. /etc/os-release && echo "$VERSION_CODENAME")
release=https://github.com/bande-linux/apt/releases/latest/download
sources=$(mktemp -d)
curl -fsSL -o "$sources/bande-archive-keyring.deb" "$release/bande-archive-keyring.deb"
curl -fsSL -o "$sources/bande-apt-source-$suite.deb" "$release/bande-apt-source-$suite.deb"
apt-get install -y -qq "$sources/bande-archive-keyring.deb" "$sources/bande-apt-source-$suite.deb" > /dev/null
apt-get update -qq
apt-get install -y -qq --no-install-recommends bande-compositor bande-config xdg-desktop-portal-bande "$deb" > /dev/null
profile=$(mktemp -d)
"$repo/scripts/capture.sh" 600 /tmp/capture.log "$repo/scripts/session.sh" timeout 598 \
    /usr/lib/bande-browser/bande-browser --no-remote --profile "$profile"
