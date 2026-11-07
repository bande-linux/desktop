#!/bin/bash
# published.sh: the bande-browser version the Bande archive serves, or nothing.
set -euo pipefail
curl -fsSL https://bande-linux.github.io/apt/dists/rolling/main/binary-arm64/Packages.xz | xz -d |
    awk '/^Package: / { p = ($2 == "bande-browser") } p && /^Version: / { print $2 }' | sort -V | tail -1
