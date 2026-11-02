#!/bin/sh
set -eu
awk '
function fail(message) { print FILENAME ":" FNR ": " message > "/dev/stderr"; failed = 1; exit 1 }
/^[ \t]*(#|$)/ { next }
{
    if (NF != 1) fail("expected one package")
    if ($1 in seen) fail("duplicate " $1)
    seen[$1] = 1
    recommends = recommends == "" ? $1 : recommends ", " $1
}
END {
    if (failed) exit 1
    print "bande:Recommends=" recommends
}
' "${1:-default-apps}"
