#!/bin/bash
# edid-corpus.sh [count]: fetches one EDID of every n-th monitor model in the linuxhw/EDID
# repository (CC-BY-4.0, https://github.com/linuxhw/EDID), about <count> (default 5000) in all,
# into ${BANDE_EDID_CORPUS:-~/.cache/bande-edid}/edids, where
# `cargo test -p bande-compositor --test edid_corpus` reads them. Run it again to re-sample.
set -euo pipefail
count=${1:-5000}
[[ $count =~ ^[1-9][0-9]*$ ]] || { echo "usage: edid-corpus.sh [count]" >&2; exit 2; }
corpus=${BANDE_EDID_CORPUS:-${XDG_CACHE_HOME:-$HOME/.cache}/bande-edid}
repo=$corpus/linuxhw

if [ ! -d "$repo/.git" ]; then
    mkdir -p "$corpus"
    git clone --quiet --depth 1 --filter=blob:none --no-checkout https://github.com/linuxhw/EDID "$repo"
fi

sample=$(mktemp)
trap 'rm -f "$sample"' EXIT
git -C "$repo" ls-tree -r -z HEAD |
    tr '\0' '\n' |
    awk -F'\t' '$2 ~ /^(Digital|Analog)\/.+\/[0-9A-F]{12}$/ { split($2, part, "/"); if (!seen[part[1] "/" part[2] "/" part[3]]++) print }' |
    awk -v count="$count" '{ line[NR] = $0 } END {
        step = NR / count; if (step < 1) step = 1
        for (at = 1; at <= NR; at += step) print line[int(at)]
    }' >"$sample"

cut -f1 "$sample" | cut -d' ' -f3 |
    git -C "$repo" -c fetch.negotiationAlgorithm=noop fetch --quiet --no-tags --no-write-fetch-head \
        --filter=blob:none --stdin origin
rm -rf "$corpus/edids"
while IFS=$'\t' read -r object path; do
    mkdir -p "$corpus/edids/${path%/*}"
    git -C "$repo" cat-file blob "${object##* }" >"$corpus/edids/$path"
done <"$sample"
echo "edid-corpus: $(wc -l <"$sample") EDIDs in $corpus/edids"
