#!/bin/bash
set -euo pipefail

repo=${1:-.}
declared=$(awk '/^\[/ { section = $0 } section == "[workspace.package]" && $1 == "version" { gsub(/"/, "", $3); print $3; exit }' "$repo/Cargo.toml")
[ -n "$declared" ] || { echo "version.sh: Cargo.toml declares no workspace version" >&2; exit 1; }

if [ "${GITHUB_REF_TYPE:-}" = tag ]; then
    tag=${GITHUB_REF_NAME:?}
    [[ $tag =~ ^v([0-9]+\.[0-9]+\.[0-9]+)$ ]] || { echo "version.sh: $tag is not a vX.Y.Z tag" >&2; exit 1; }
    [ "${BASH_REMATCH[1]}" = "$declared" ] ||
        { echo "version.sh: tag $tag but Cargo.toml says $declared" >&2; exit 1; }
    echo "$declared"
    exit 0
fi

last=$(git -C "$repo" describe --tags --abbrev=0 --match 'v[0-9]*.[0-9]*.[0-9]*' HEAD 2>/dev/null || true)
if [ -n "$last" ]; then
    base=${last#v}
    count=$(git -C "$repo" rev-list --count "$last..HEAD")
else
    base=0.0.0
    count=$(git -C "$repo" rev-list --count HEAD)
fi
date=$(TZ=UTC git -C "$repo" log -1 --format=%cd --date=format-local:%Y%m%d)
sha=$(git -C "$repo" rev-parse --short=7 HEAD)
echo "$base+git$date.$count.$sha"
