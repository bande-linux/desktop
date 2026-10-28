#!/bin/bash
set -euo pipefail

images=$(realpath "${1:?usage: guest-tests.sh <images-dir>}")
here=$(dirname "$(realpath "$0")")
share=$(mktemp -d)
trap 'rm -rf "$share"' EXIT

cargo test --workspace --locked --no-run --message-format=json \
    | jq -r 'select(.reason == "compiler-artifact" and .target.name == "guest" and .executable != null) | .executable' \
    > "$share/executables"
if [ ! -s "$share/executables" ]; then
    echo "No guest test executables were built" >&2
    exit 1
fi

mkdir "$share/bin"
while read -r executable; do
    cp "$executable" "$share/bin/$(basename "$executable")"
done < "$share/executables"

cat > "$share/run" <<'GUEST'
set -u
echo "Guest kernel $(uname -r)"
ls -l /dev/dri
status=0
for test in bin/*; do
    echo "Running ${test#bin/} in the guest"
    "$test" --ignored --test-threads=1 || status=1
done
exit "$status"
GUEST

"$here/run.sh" "$images" "$share"
