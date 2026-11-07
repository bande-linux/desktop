#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/../.."

cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings

forbid() {
    local found
    found=$(cargo tree --locked --manifest-path "$1" --edges normal --prefix none | grep -E "^($2) " || true)
    [ -z "$found" ] || { echo "$found"; echo "$1 must not depend on the crates above"; exit 1; }
}
for manifest in crates/services/*/Cargo.toml; do
    [ "$manifest" = crates/services/polkit/Cargo.toml ] && continue
    forbid "$manifest" 'bande-widgets|bande-toolkit|bande-toolkit-platform'
done
for manifest in crates/toolkit/graphics/Cargo.toml crates/toolkit/widgets/Cargo.toml; do
    forbid "$manifest" 'wayland-[a-z-]+|bande-toolkit-platform'
done
if grep -rnE '\.actions\b|Desktop Action|DesktopAction' crates/apps/files/src crates/shell/desktop/src \
    || grep -nE 'DesktopEntry' crates/apps/files/src/menus.rs crates/shell/desktop/src/menus.rs; then
    echo "no application adds entries to the context menus of Files or the desktop: see docs/architecture/behaviour.md"
    exit 1
fi
ci/check-apps.sh
echo "lint passed"
