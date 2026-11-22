#!/bin/bash
# check-apps.sh [app dir...]: static checks (no build) that each app (default: crates/apps/* but the
# file chooser and the keyring prompt) is a bande-app app packaged like every other: the package is
# bande-<dir>; the app id in src/main.rs names the .desktop file, its Icon= and the metainfo <id>;
# Exec= starts the package; StartupNotify=true; every MimeType= it declares has a default in
# data/x-bande-mimeapps.list; its debian/control Depends has the toolkit's three
# font packages (the template, which is not released, has no stanza);
# src/main.rs calls bande_app::run and nothing in src/ runs its own loop or menu export. Fails on
# any problem.
set -euo pipefail
cd "$(dirname "$0")/.."

problems=0
problem() {
    echo "check-apps: $1: $2" >&2
    problems=$((problems + 1))
}

check() {
    local dir=${1%/} package id desktop metainfo
    package=$(sed -n 's/^name = "\(.*\)"$/\1/p' "$dir/Cargo.toml" | head -1)
    [ "$package" = "bande-$(basename "$dir")" ] || problem "$dir" "the package is $package, not bande-$(basename "$dir")"
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
        for mime in $(sed -n 's/^MimeType=//p' "$desktop" | tr ';' ' '); do
            awk -F= -v mime="$mime" '$1 == mime { found = 1 } END { exit !found }' data/x-bande-mimeapps.list ||
                problem "$dir" "$mime has no default in data/x-bande-mimeapps.list"
        done
    fi
    if [ -f "$metainfo" ]; then
        grep -q "<id>$id</id>" "$metainfo" || problem "$dir" "the metainfo <id> is not $id"
    fi
    depends=$(awk -v package="$package" '/^Package: / { found = $2 == package } found && /^[A-Z]/ { field = $1 }
        found && field == "Depends:"' debian/control)
    if [ "$package" != bande-template ]; then
        [ -n "$depends" ] || problem "$dir" "debian/control has no Depends of $package"
        for font in fonts-inter fonts-manrope fonts-jetbrains-mono; do
            grep -qE "(^|[ :])$font( |,|$)" <<< "$depends" || problem "$dir" "Depends lacks $font"
        done
    fi
    grep -q "bande_app::run::<" "$dir/src/main.rs" || problem "$dir" "src/main.rs does not call bande_app::run"
    if grep -rqE "AppMenu::export|bande_toolkit::run\(|run_surfaces" "$dir/src"; then
        problem "$dir" "src/ runs its own loop or menu export instead of bande-app"
    fi
}

apps=("$@")
if [ $# -eq 0 ]; then
    for dir in crates/apps/*/; do
        case $dir in crates/apps/file-chooser/ | crates/apps/secrets-prompt/) continue ;; esac
        apps+=("$dir")
    done
fi
for app in "${apps[@]}"; do
    check "$app"
done
[ "$problems" -eq 0 ] || { echo "check-apps: $problems problems" >&2; exit 1; }
echo "check-apps passed: ${#apps[@]} apps"
