#!/bin/bash
# new-app.sh <dir> <Id> "<Name>": a new app in crates/apps/<dir>, copied from crates/apps/template:
# the package bande-<dir>, the app id org.bande.<Id> and the name <Name>, e.g.
# `ci/new-app.sh mail-merge MailMerge "Mail Merge"`. Then checks it with ci/check-apps.sh.
set -euo pipefail
cd "$(dirname "$0")/.."
dir=${1:?usage: new-app.sh <dir> <Id> "<Name>"} id=${2:?usage: new-app.sh <dir> <Id> "<Name>"}
name=${3:?usage: new-app.sh <dir> <Id> "<Name>"}
[[ $dir =~ ^[a-z][a-z0-9-]*$ ]] || { echo "new-app: $dir is not a lower-case directory name" >&2; exit 1; }
[[ $id =~ ^[A-Z][A-Za-z0-9]*$ ]] || { echo "new-app: $id is not an id like MailMerge" >&2; exit 1; }
target=crates/apps/$dir
[ ! -e "$target" ] || { echo "new-app: $target exists" >&2; exit 1; }
cp -r crates/apps/template "$target"
for file in "$target"/data/org.bande.Template.*; do
    mv "$file" "${file/org.bande.Template/org.bande.$id}"
done
find "$target" -type f -exec sed -i \
    -e "s/bande-template/bande-$dir/g" \
    -e "s/org\.bande\.Template/org.bande.$id/g" {} +
find "$target/src" -type f -exec sed -i "s/Template/$id/g" {} +
find "$target" -path "$target/src" -prune -o -type f -exec sed -i "s/Template/$name/g" {} +
cargo metadata -q --format-version 1 > /dev/null
ci/check-apps.sh "$target"
cat <<NEXT
new-app: $target is bande-$dir (org.bande.$id). Next:
- describe it: description and extended-description in Cargo.toml, the metainfo
- draw its Bande theme icon in data/icons/src/apps.py
- release it: add bande-$dir to ci/release/packages and the apps table in README.md
- if it is a default app, add it to bande-desktop's Recommends in packaging/bande-desktop/debian/control
- compile it: cargo check --locked -p bande-$dir
NEXT
