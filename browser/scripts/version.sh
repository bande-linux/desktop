#!/bin/bash
# version.sh <debian version>: our version of it, <debian version>+bandeN, where N counts this
# repository's commits, so any change here gives a newer package.
set -euo pipefail
repo=$(dirname "$(dirname "$(realpath "$0")")")
echo "${1:?usage: version.sh <debian version>}+bande$(git -C "$repo" rev-list --count HEAD)"
