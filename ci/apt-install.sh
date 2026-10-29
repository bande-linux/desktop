#!/bin/bash
# Installs the given Debian packages, skipping apt-get entirely when the runner image
# already carries all of them.
set -euo pipefail

missing=()
for package in "$@"; do
    status=$(dpkg-query -W -f='${db:Status-Status}' "$package" 2>/dev/null || true)
    [ "$status" = installed ] || missing+=("$package")
done
[ "${#missing[@]}" -eq 0 ] && exit 0
echo "Installing ${missing[*]}"
sudo apt-get update
sudo apt-get install -y --no-install-recommends "${missing[@]}"
