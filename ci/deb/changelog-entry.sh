#!/bin/bash
# changelog-entry.sh <changelog> <version> <distribution> <text>: adds an entry for <version> on
# top of a debian/changelog whose newest entry is another version, by its maintainer and dated
# $SOURCE_DATE_EPOCH, so the same build writes the same entry.
set -euo pipefail
changelog=$1 version=$2 distribution=$3 text=$4
[ "$(dpkg-parsechangelog -l "$changelog" -S Version)" != "$version" ] || exit 0
maintainer=$(dpkg-parsechangelog -l "$changelog" -S Maintainer)
date=$(date -R -u -d "@${SOURCE_DATE_EPOCH:?}")
{
    printf 'bande-desktop (%s) %s; urgency=medium\n\n  * %s\n\n -- %s  %s\n\n' \
        "$version" "$distribution" "$text" "$maintainer" "$date"
    cat "$changelog"
} > "$changelog.new"
mv "$changelog.new" "$changelog"
