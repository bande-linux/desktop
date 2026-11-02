#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/../.."

repo=bande-linux/desktop
git fetch -q origin main
commit=$(git rev-parse "${1:-origin/main}^{commit}")
[ "$commit" = "$(git rev-parse origin/main)" ] ||
    { echo "dry-run: a dispatch runs on main's head $(git rev-parse --short origin/main), not $commit" >&2; exit 1; }
version=$(git show "$commit:Cargo.toml" |
    awk '/^\[/ { section = $0 } section == "[workspace.package]" && $1 == "version" { gsub(/"/, "", $3); print $3; exit }')
tag=v$version
[ -z "$(git ls-remote --tags origin "refs/tags/$tag")" ] || { echo "dry-run: $tag is already tagged" >&2; exit 1; }

since=$(date -u -d "-1 minute" +%Y-%m-%dT%H:%M:%SZ)
gh workflow run release.yml --repo "$repo" --ref main -f tag="$tag"
run=
for _ in $(seq 30); do
    run=$(gh run list --repo "$repo" --workflow release.yml --event workflow_dispatch --commit "$commit" \
        --created ">=$since" --json databaseId --jq '.[0].databaseId // empty')
    [ -z "$run" ] || break
    sleep 5
done
[ -n "$run" ] || { echo "dry-run: the dispatched run did not appear" >&2; exit 1; }
url=$(gh run view "$run" --repo "$repo" --json url --jq .url)
echo "dry-run: $tag on $(git rev-parse --short "$commit"): $url"
gh run watch "$run" --repo "$repo" --exit-status --interval 60 > /dev/null ||
    { echo "dry-run: $tag is red: $url" >&2; exit 1; }
echo "dry-run: $tag is green on $commit; tag it with: git tag $tag $commit && git push origin $tag"
