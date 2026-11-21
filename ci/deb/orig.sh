#!/bin/bash
# orig.sh <out dir> <upstream version>: the upstream tarball bande-desktop_<version>.orig.tar.xz,
# git archive of HEAD (debian/ and the other export-ignore paths left out), byte for byte the same
# for the same commit.
set -euo pipefail
cd "$(dirname "$0")/../.."
out=$(realpath "${1:?usage: orig.sh <out dir> <upstream version>}")
version=${2:?usage: orig.sh <out dir> <upstream version>}
mkdir -p "$out"
git archive --format=tar --prefix="bande-desktop-$version/" HEAD |
    xz --threads=1 -6 > "$out/bande-desktop_$version.orig.tar.xz"
