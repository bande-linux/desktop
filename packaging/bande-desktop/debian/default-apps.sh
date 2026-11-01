#!/bin/sh
set -eu
awk '
function fail(message) { print FILENAME ":" FNR ": " message > "/dev/stderr"; failed = 1; exit 1 }
function add(list, name) { return list == "" ? name : list ", " name }
/^[ \t]*(#|$)/ { next }
/^\[/ {
    if ($0 != "[apps]" && $0 != "[debian]") fail("unknown section " $0)
    section = $0
    next
}
{
    if (section == "") fail("package outside a section")
    if (NF != 3) fail("expected package, icon and relation")
    if ($1 in seen) fail("duplicate " $1)
    seen[$1] = 1
    if ($3 == "recommends") recommends = add(recommends, $1)
    else if ($3 == "suggests") suggests = add(suggests, $1)
    else fail("unknown relation " $3)
}
END {
    if (failed) exit 1
    print "bande:Recommends=" recommends
    print "bande:Suggests=" suggests
}
' "${1:-default-apps}"
