#!/bin/bash
# debian-version.sh: the newest firefox-esr source in Debian unstable. Testing lags it by days,
# and those days carry open security fixes, so we follow unstable.
set -euo pipefail
curl -fsSL https://deb.debian.org/debian/dists/unstable/main/source/Sources.xz | xz -d |
    awk '/^Package: / { p = ($2 == "firefox-esr") } p && /^Version: / { print $2 }' | sort -V | tail -1
