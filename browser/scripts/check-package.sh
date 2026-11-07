#!/bin/bash
# check-package.sh <deb>: as root in a Debian testing container with the Bande archive: installs the
# package and proves a fresh profile stays silent for 10 minutes. No release passes without this.
set -euo pipefail
deb=$(realpath "$1")
repo=$(dirname "$(dirname "$(realpath "$0")")")
export DEBIAN_FRONTEND=noninteractive
apt-get update -qq
apt-get install -y -qq --no-install-recommends gpg ca-certificates curl tcpdump iproute2 python3 dbus \
    mesa-vulkan-drivers > /dev/null
curl -fsSL https://bande-linux.github.io/apt/archive-key.asc | gpg --dearmor > /usr/share/keyrings/bande.gpg
printf 'Types: deb\nURIs: https://bande-linux.github.io/apt\nSuites: rolling\nComponents: main\nSigned-By: /usr/share/keyrings/bande.gpg\n' \
    > /etc/apt/sources.list.d/bande.sources
apt-get update -qq
apt-get install -y -qq --no-install-recommends bande-compositor bande-config xdg-desktop-portal-bande "$deb" > /dev/null
profile=$(mktemp -d)
"$repo/scripts/capture.sh" 600 /tmp/capture.log "$repo/scripts/session.sh" timeout 598 \
    /usr/lib/bande-browser/bande-browser --no-remote --profile "$profile"
