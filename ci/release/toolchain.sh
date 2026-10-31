#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/../.."

channel=$(sed -n 's/^channel = "\(.*\)"$/\1/p' rust-toolchain.toml)
components=$(sed -n 's/^components = \[\(.*\)\]$/\1/p' rust-toolchain.toml | tr -d '" ')
rustup toolchain install "$channel" --profile minimal --component "$components"
rustup default "$channel"
cargo --version
