#!/bin/bash
# published.sh [suite]: the bande-browser release the Bande archive serves in that suite (default
# forky), without its Debian release suffix (~debNN), or nothing when it serves none. Fails when
# the archive cannot be read.
set -euo pipefail
url=https://bande-linux.github.io/apt/dists/${1:-forky}/main/binary-amd64/Packages.xz
index=$(curl -fsSL --retry 3 "$url" | xz -d) || { echo "published: cannot read $url" >&2; exit 1; }
grep -q '^Package: ' <<< "$index" || { echo "published: $url lists no package" >&2; exit 1; }
awk '/^Package: / { p = ($2 == "bande-browser") } p && /^Version: / { print $2 }' <<< "$index" | sort -V | tail -1 |
    sed 's/~deb[0-9][0-9]*\(+.*\)\{0,1\}$//'
