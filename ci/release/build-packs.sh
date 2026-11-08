#!/bin/bash
# build-packs.sh <out> <version>: the language packs, bande-desktop-l10n-<lang>, checked against the
# code's English and built from the translations alone. ci/release/build-all.sh and the
# translation-only l10n-YYYYMMDD releases (.github/workflows/l10n.yml) run it.
set -euo pipefail
cd "$(dirname "$0")/../.."

out=$(realpath -m "${1:?usage: build-packs.sh <out> <version>}")
version=${2:?usage: build-packs.sh <out> <version>}
target=${CARGO_TARGET_DIR:-target}
cargo build --release --locked --package=bande-l10n-tool
tool=$target/release/bande-l10n-tool
"$tool" check ci/l10n-complete
rm -rf "$target/l10n"
"$tool" pack "$target/l10n"
"$tool" debs "$target/l10n" "$out" "$version" bande-desktop packaging/bande-desktop/debian/copyright
