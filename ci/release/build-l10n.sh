#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/../.."

catalogs=$(realpath "${1:?usage: build-l10n.sh <catalogs from bande-l10n-tool pack> <out> <version> <package prefix> <enhances>}")
out=$(realpath -m "${2:?}")
version=${3:?}
prefix=${4:?}
enhances=${5:?}
maintainer="Mikael Hug <mikaelhug@gmail.com>"
mkdir -p "$out"
scratch=$(mktemp -d)
trap 'rm -rf "$scratch"' EXIT

language_team() {
    find crates -path "*/locales/$1.po" -print -quit | xargs sed -n 's/^"Language-Team: \(.*\)\\n"$/\1/p' | head -n1
}

for folder in "$catalogs"/packs/*/; do
    language=$(basename "$folder")
    package="$prefix-l10n-${language,,}"
    team=$(language_team "$language")
    [ -n "$team" ] || { echo "build-l10n.sh: no Language-Team in the $language translations" >&2; exit 1; }
    root="$scratch/$package"
    mkdir -p "$root/DEBIAN" "$root/usr/share/bande/l10n" "$root/usr/share/doc/$package"
    cp -r "$folder"/. "$root/usr/share/bande/l10n/"
    cp packaging/bande-desktop/debian/copyright "$root/usr/share/doc/$package/copyright"
    printf '%s (%s) unstable; urgency=medium\n\n  * Built from the translations at this version.\n\n -- %s  %s\n' \
        "$package" "$version" "$maintainer" "$(date -R -d "@${SOURCE_DATE_EPOCH:-$(git log -1 --format=%ct)}")" |
        gzip -9n > "$root/usr/share/doc/$package/changelog.gz"
    find "$root/usr" -type d -exec chmod 755 {} + -o -type f -exec chmod 644 {} +
    size=$(du -sk --exclude=DEBIAN "$root" | cut -f1)
    cat > "$root/DEBIAN/control" << EOF
Package: $package
Version: $version
Architecture: all
Maintainer: $maintainer
Installed-Size: $size
Enhances: $enhances
Section: localization
Priority: optional
Multi-Arch: foreign
Description: $team translations for $enhances
 The $team text of every $enhances program, read by each program when it
 starts in a $team session. Without it those programs show English.
EOF
    (cd "$root" && find usr -type f -print0 | sort -z | xargs -0 md5sum) > "$root/DEBIAN/md5sums"
    dpkg-deb --root-owner-group -Zxz --build "$root" "$out/${package}_${version}_all.deb"
done
