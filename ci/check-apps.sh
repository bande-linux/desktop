#!/bin/bash
# check-apps.sh [app dir...]: static checks that each desktop app is a bande-app app with matching
# packaging (no build). Warns for now; fails once every app has moved onto bande-app.
set -euo pipefail
cd "$(dirname "$0")/.."

problems=0
problem() {
    echo "check-apps: $1: $2" >&2
    problems=$((problems + 1))
}

check() {
    local dir=$1 package id desktop metainfo
    package=$(sed -n 's/^name = "\(.*\)"$/\1/p' "$dir/Cargo.toml" | head -1)
    id=$(sed -n 's/^ *id: "\([^"]*\)",$/\1/p; s/^const APP_ID: &str = "\([^"]*\)";$/\1/p' "$dir/src/main.rs" | head -1)
    [ -n "$id" ] || { problem "$dir" "no app id in src/main.rs"; return; }
    desktop="$dir/data/$id.desktop"
    metainfo="$dir/data/$id.metainfo.xml"
    [ -f "$desktop" ] || problem "$dir" "no data/$id.desktop"
    [ -f "$metainfo" ] || problem "$dir" "no data/$id.metainfo.xml"
    if [ -f "$desktop" ]; then
        grep -qx "Icon=$id" "$desktop" || problem "$dir" "Icon= is not $id"
        grep -q "^Exec=$package\( \|$\)" "$desktop" || problem "$dir" "Exec= does not start $package"
        grep -qx "StartupNotify=true" "$desktop" || problem "$dir" "no StartupNotify=true"
    fi
    if [ -f "$metainfo" ]; then
        grep -q "<id>$id</id>" "$metainfo" || problem "$dir" "the metainfo <id> is not $id"
    fi
    for font in fonts-inter fonts-manrope fonts-jetbrains-mono; do
        grep -q "^depends = .*\b$font\b" "$dir/Cargo.toml" || problem "$dir" "Depends lacks $font"
    done
    grep -q "bande_app::run::<" "$dir/src/main.rs" || problem "$dir" "src/main.rs does not call bande_app::run"
    if grep -rqE "AppMenu::export|bande_toolkit::run\(|run_surfaces" "$dir/src"; then
        problem "$dir" "src/ runs its own loop or menu export instead of bande-app"
    fi
}

apps=("$@")
[ $# -gt 0 ] || apps=(crates/apps/files crates/apps/settings)
for app in "${apps[@]}"; do
    check "$app"
done
[ "$problems" -eq 0 ] && echo "check-apps passed" || echo "check-apps: $problems problems (warnings until every app is on bande-app)"
