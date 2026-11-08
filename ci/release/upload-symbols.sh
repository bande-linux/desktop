#!/bin/bash
set -euo pipefail

deb=${1:?usage: upload-symbols.sh <bande-debug-symbols deb> <endpoint>}
endpoint=${2:?usage: upload-symbols.sh <bande-debug-symbols deb> <endpoint>}
endpoint=${endpoint%/}
[ -n "${LOGS_SYMBOLS_KEY:-}" ] || { echo "upload-symbols: LOGS_SYMBOLS_KEY is not set" >&2; exit 2; }

scratch=$(mktemp -d)
trap 'rm -rf "$scratch"' EXIT
dpkg-deb -x "$deb" "$scratch/root"
printf 'Authorization: Bearer %s\n' "$LOGS_SYMBOLS_KEY" > "$scratch/header"

sent=0 failed=0
while IFS= read -r -d '' debug; do
    relative=${debug#"$scratch/root/usr/lib/debug/.build-id/"}
    id=${relative%.debug}
    id=${id/\//}
    [[ $id =~ ^[0-9a-f]{16,}$ ]] || { echo "upload-symbols: skipping $relative" >&2; continue; }
    status=$(curl --silent --show-error --output /dev/null --write-out '%{http_code}' \
        --retry 3 --max-time 300 -H @"$scratch/header" \
        -H 'Content-Type: application/octet-stream' --upload-file "$debug" \
        "$endpoint/v1/symbols/$id") || status=000
    case $status in
        2??) sent=$((sent + 1)) ;;
        *) echo "upload-symbols: $id answered $status" >&2; failed=$((failed + 1)) ;;
    esac
done < <(find "$scratch/root/usr/lib/debug/.build-id" -name '*.debug' -type f -print0)

echo "upload-symbols: $sent sent, $failed failed"
[ "$failed" = 0 ]
