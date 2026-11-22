#!/bin/bash
# inputs.sh: a hash of every file in browser/ that goes into the packages, which is all of it but
# the Markdown docs; the same hash means the same build.
set -euo pipefail
cd "$(dirname "$0")/.."
listing=$(git ls-tree -r HEAD -- .)
[ -n "$listing" ]
grep -v '\.md$' <<< "$listing" | git hash-object --stdin
