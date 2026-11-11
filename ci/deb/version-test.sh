#!/bin/bash
set -euo pipefail

version=$(realpath "$(dirname "$0")/version.sh")
repo=$(mktemp -d)
git -C "$repo" init -q -b main
git -C "$repo" config user.email test@example.org
git -C "$repo" config user.name test
failures=0
printf 'ID=debian\nVERSION_CODENAME=forky\n' > "$repo.forky"
printf 'ID=debian\nVERSION_CODENAME=duke\n' > "$repo.duke"
trap 'rm -rf "$repo" "$repo.forky" "$repo.duke"' EXIT
export OS_RELEASE=$repo.forky

commit() {
    printf '[workspace.package]\nversion = "%s"\n' "$1" > "$repo/Cargo.toml"
    GIT_COMMITTER_DATE="$2" GIT_AUTHOR_DATE="$2" git -C "$repo" commit -q --allow-empty -am "$2" 2>/dev/null ||
        { git -C "$repo" add Cargo.toml; GIT_COMMITTER_DATE="$2" GIT_AUTHOR_DATE="$2" git -C "$repo" commit -q -m "$2"; }
}
build() { env -u GITHUB_REF_TYPE "$version" "$repo"; }
expect() {
    if [ "$1" = "$2" ]; then echo "ok   $3"; else echo "FAIL $3: got $1, want $2"; failures=$((failures + 1)); fi
}

commit 0.1.0 '2026-10-29T08:00:00Z'
first=$(build)
expect "$first" "0.0.0+git20261029.1.$(git -C "$repo" rev-parse --short=7 HEAD)~deb14" "a build before any tag counts from 0.0.0"
commit 0.1.0 '2026-10-29T09:00:00Z'
same_day=$(build)
git -C "$repo" tag v0.1.0
release=$(GITHUB_REF_TYPE=tag GITHUB_REF_NAME=v0.1.0 "$version" "$repo")
expect "$release" 0.1.0~deb14 "a vX.Y.Z tag gives X.Y.Z~debNN"
at_tag=$(build)
commit 0.1.0 '2026-10-30T07:00:00Z'
after=$(build)
commit 0.1.0 '2026-10-30T08:00:00Z'
later=$(build)
expect "${later%.*}" "0.1.0+git20261030.2" "builds after a tag count from it"
commit 0.1.1 '2026-10-31T08:00:00Z'
git -C "$repo" tag v0.1.1
next_release=$(GITHUB_REF_TYPE=tag GITHUB_REF_NAME=v0.1.1 "$version" "$repo")

git -C "$repo" tag l10n-20261031
translations=$(GITHUB_REF_TYPE=tag GITHUB_REF_NAME=l10n-20261031 "$version" "$repo")
expect "$translations" 0.1.1~deb14+l10n20261031 "an l10n-YYYYMMDD tag counts from the last release"
second=$(GITHUB_REF_TYPE=tag GITHUB_REF_NAME=l10n-20261031.2 "$version" "$repo")
commit 0.1.2 '2026-11-01T08:00:00Z'
git -C "$repo" tag v0.1.2
following=$(GITHUB_REF_TYPE=tag GITHUB_REF_NAME=v0.1.2 "$version" "$repo")
# Upgrading Debian upgrades Bande: the next release's build of the same version sorts above
# every build for the previous one, translation packs included.
next_debian=$(OS_RELEASE=$repo.duke GITHUB_REF_TYPE=tag GITHUB_REF_NAME=v0.1.2 "$version" "$repo")
expect "$next_debian" 0.1.2~deb15 "the build for the next Debian release carries its number"
next_translations=$(GITHUB_REF_TYPE=tag GITHUB_REF_NAME=l10n-20261101 "$version" "$repo")

previous=
for v in "$first" "$same_day" "$release" "$at_tag" "$after" "$later" "$next_release" "$translations" "$second" "$following" "$next_translations" "$next_debian"; do
    if [ -n "$previous" ]; then
        if dpkg --compare-versions "$previous" lt "$v"; then
            echo "ok   $previous < $v"
        else
            echo "FAIL $previous is not below $v"
            failures=$((failures + 1))
        fi
    fi
    previous=$v
done

if GITHUB_REF_TYPE=tag GITHUB_REF_NAME=v0.2.0 "$version" "$repo" 2>/dev/null; then
    echo "FAIL a tag that disagrees with Cargo.toml was accepted"; failures=$((failures + 1))
else
    echo "ok   a tag that disagrees with Cargo.toml is refused"
fi
if GITHUB_REF_TYPE=tag GITHUB_REF_NAME=v0.1 "$version" "$repo" 2>/dev/null; then
    echo "FAIL a tag that is not vX.Y.Z was accepted"; failures=$((failures + 1))
else
    echo "ok   a tag that is not vX.Y.Z is refused"
fi

[ "$failures" = 0 ] || { echo "$failures failed"; exit 1; }
