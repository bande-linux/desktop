#!/bin/bash
# published.sh [suite]: the bande-browser release the Bande archive serves in that suite (default
# forky), without its Debian release suffix (~debNN), or nothing.
set -euo pipefail
curl -fsSL "https://bande-linux.github.io/apt/dists/${1:-forky}/main/binary-arm64/Packages.xz" | xz -d |
    awk '/^Package: / { p = ($2 == "bande-browser") } p && /^Version: / { print $2 }' | sort -V | tail -1 |
    sed 's/~deb[0-9][0-9]*\(+.*\)\{0,1\}$//'
