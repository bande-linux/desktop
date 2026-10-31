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
    forbid "$manifest" 'unbound-widgets|unbound-toolkit|unbound-toolkit-platform'
done
for manifest in crates/toolkit/graphics/Cargo.toml crates/toolkit/widgets/Cargo.toml; do
    forbid "$manifest" 'wayland-[a-z-]+|unbound-toolkit-platform'
done
echo "lint passed"
