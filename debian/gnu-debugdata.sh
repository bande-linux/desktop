#!/bin/bash
# gnu-debugdata.sh names|add <dir> <package>...: keeps a small table of function names in each
# program, so a crash report has a stack trace with names on every computer. "names", before
# dh_strip, writes the names of each ELF file's local functions, xz-compressed, to <dir>;
# "add", after it, puts them into the stripped file as .gnu_debugdata.
set -euo pipefail
mode=$1 dir=$2
shift 2
scratch=$(mktemp -d)
trap 'rm -rf "$scratch"' EXIT

for package; do
    while IFS= read -r -d '' file; do
        [ "$(od -An -tx1 -N4 "$file" | tr -d ' ')" = 7f454c46 ] || continue
        names=$dir/$file.xz
        case $mode in
            names)
                mkdir -p "$(dirname "$names")"
                nm --dynamic --format=posix --defined-only "$file" | cut -d' ' -f1 | sort -u > "$scratch/exported"
                nm --format=posix --defined-only "$file" | awk '$2 == "T" || $2 == "t" { print $1 }' | sort -u > "$scratch/functions"
                comm -13 "$scratch/exported" "$scratch/functions" > "$scratch/kept"
                objcopy --only-keep-debug "$file" "$scratch/whole"
                objcopy --strip-debug --remove-section=.comment --keep-symbols="$scratch/kept" "$scratch/whole" "$scratch/names"
                xz --stdout --threads=1 "$scratch/names" > "$names"
                ;;
            add)
                objcopy --add-section .gnu_debugdata="$names" "$file"
                ;;
            *) echo "gnu-debugdata.sh: no mode $mode" >&2; exit 2 ;;
        esac
    done < <(find "debian/$package" -path "debian/$package/usr/lib/debug" -prune -o -type f -print0 | sort -z)
done
