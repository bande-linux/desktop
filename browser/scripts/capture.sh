#!/bin/bash
# capture.sh <seconds> <log> <command...>: runs the command in an empty network namespace whose only
# route leads to a dummy interface, and logs every packet leaving it. Nothing answers, so a phone-home
# shows up as its first DNS query. Run as root in a user namespace. Exit 0 only if nothing left.
set -euo pipefail
seconds=$1 log=$2; shift 2
exec unshare --net bash -c '
    seconds=$0 log=$1; shift
    ip link set lo up
    ip link add dummy0 type dummy
    ip addr add 10.0.2.100/24 dev dummy0
    ip link set dummy0 up
    ip route add default via 10.0.2.2
    echo "nameserver 10.0.2.3" > /etc/resolv.conf
    tcpdump -Z root -l -n -i dummy0 -s 512 -v > "$log" 2> "$log.err" &
    dump=$!
    sleep 1
    "$@" &
    app=$!
    sleep "$seconds"
    kill $app 2>/dev/null || true
    sleep 1
    kill $dump
    wait 2>/dev/null || true
    echo "capture: $(grep -c "IP" "$log" || true) packets"
    grep -oE "A\? [a-z0-9.-]+|AAAA\? [a-z0-9.-]+|HTTPS\? [a-z0-9.-]+" "$log" | sed "s/.*? //" | sort | uniq -c | sort -rn
    ! grep -vE "^\s|^$|igmp|IGMP|ICMP6|ip6-allrouters|ff02::" "$log" | grep -q "IP "
' "$seconds" "$log" "$@"
