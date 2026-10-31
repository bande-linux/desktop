#!/bin/bash
set -euo pipefail

missing=()
for package in "$@"; do
    status=$(dpkg-query -W -f='${db:Status-Status}' "$package" 2>/dev/null || true)
    [ "$status" = installed ] || missing+=("$package")
done
[ "${#missing[@]}" -eq 0 ] && exit 0
echo "Installing ${missing[*]}"
as_root=()
[ "$(id -u)" = 0 ] || as_root=(sudo)
"${as_root[@]}" apt-get update
"${as_root[@]}" env DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends "${missing[@]}"
