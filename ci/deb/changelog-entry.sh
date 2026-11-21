#!/bin/bash
# changelog-entry.sh <changelog> <version> <distribution> <text>: adds an entry for <version> on
# top of a debian/changelog whose newest entry is another version, by its maintainer and dated
# $SOURCE_DATE_EPOCH or a second after that entry, whichever is later, so the same build writes
# the same entry.
set -euo pipefail
changelog=$1 version=$2 distribution=$3 text=$4
[ "$(dpkg-parsechangelog -l "$changelog" -S Version)" != "$version" ] || exit 0
: "${SOURCE_DATE_EPOCH:?}"
source=$(dpkg-parsechangelog -l "$changelog" -S Source)
maintainer=$(dpkg-parsechangelog -l "$changelog" -S Maintainer)
previous=$(dpkg-parsechangelog -l "$changelog" -S Timestamp)
epoch=$((SOURCE_DATE_EPOCH > previous ? SOURCE_DATE_EPOCH : previous + 1))
date=$(date -R -u -d "@$epoch")
{
    printf '%s (%s) %s; urgency=medium\n\n  * %s\n\n -- %s  %s\n\n' "$source" \
        "$version" "$distribution" "$text" "$maintainer" "$date"
    cat "$changelog"
} > "$changelog.new"
mv "$changelog.new" "$changelog"
