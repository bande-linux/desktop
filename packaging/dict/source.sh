#!/bin/bash
# source.sh <YYYYMMDD> <sources> <out dir>: the bande-dict source package <YYYYMMDD>-1 in <out dir>:
# build.py and its pins as the upstream tarball, one component tarball per pinned source file
# (build.py fetch puts them in <sources>/<component>/) and debian/, with a debian/changelog entry
# for that version.
set -euo pipefail
cd "$(dirname "$0")"
version=${1:?usage: source.sh <YYYYMMDD> <sources> <out dir>}
sources=$(realpath "${2:?usage: source.sh <YYYYMMDD> <sources> <out dir>}")
out=$(realpath -m "${3:?usage: source.sh <YYYYMMDD> <sources> <out dir>}")
export SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH:-$(git log -1 --format=%ct -- .)}
python3 build.py fetch "$sources"
tree=$out/bande-dict-$version
rm -rf "$tree"
mkdir -p "$tree"
pack() { tar --create --sort=name --mtime="@$SOURCE_DATE_EPOCH" --owner=0 --group=0 --numeric-owner "$@" | xz --threads=1 -6; }
cp build.py sources "$tree/"
pack --directory "$out" "bande-dict-$version" > "$out/bande-dict_$version.orig.tar.xz"
while read -r name _; do
    [[ $name = \#* ]] && continue
    component=$(python3 -c 'import re, sys; print(re.sub(r"[._]", "-", sys.argv[1]).lower())' "$name")
    pack --directory "$sources" "$component/$name" > "$out/bande-dict_$version.orig-$component.tar.xz"
    mkdir -p "$tree/$component"
    cp "$sources/$component/$name" "$tree/$component/"
done < sources
cp -r debian "$tree/"
"$(git rev-parse --show-toplevel)/ci/deb/changelog-entry.sh" "$tree/debian/changelog" "$version-1" unstable \
    "Dictionary packs of $version."
(cd "$out" && dpkg-source --build "bande-dict-$version")
rm -rf "$tree"
