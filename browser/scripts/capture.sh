#!/bin/bash
# capture.sh <seconds> <log> <command...>: runs the command in an empty network namespace whose only
# route leads to a dummy interface, and logs every packet leaving it. Nothing answers, so a phone-home
# shows up as its first DNS query. Run as root in a user namespace. Exit 0 only if nothing left.
# capture.sh --live <seconds> <log> <command...>: the same with real internet through slirp4netns, for
# a browsing run: every DNS name looked up is printed, and the exit code is the command's.
set -euo pipefail
live=
if [ "$1" = --live ]; then live=1; shift; fi
seconds=$1 log=$2; shift 2
report() {
    echo "capture: $(grep -cE ' IP6? ' "$log" || true) packets"
    grep -oE '(A|AAAA|HTTPS)\? [a-z0-9.-]+' "$log" | sed 's/.*? //' | sort | uniq -c | sort -rn
}
if [ -n "$live" ]; then
    unshare --net sleep infinity &
    holder=$!
    sleep 0.5
    target=$(pgrep -P "$holder" sleep || echo "$holder")
    slirp4netns --configure --mtu=65520 "$target" tap0 > "$log.slirp" 2>&1 &
    slirp=$!
    sleep 1
    echo "nameserver 10.0.2.3" > /etc/resolv.conf
    set +e
    nsenter -t "$target" -n bash -c '
        ip link set lo up
        tcpdump -Z root -l -n -i tap0 -s 512 "port 53" > "$0" 2> "$0.err" &
        dump=$!
        sleep 1
        timeout "$1" "${@:2}"
        status=$?
        sleep 1
        kill $dump
        exit $status
    ' "$log" "$seconds" "$@"
    status=$?
    kill "$slirp" "$holder" 2>/dev/null
    report
    exit "$status"
fi
export -f report
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
    report
    ! grep -E " IP6? " "$log" | grep -vqE "ICMP6|igmp|ff02::"
' "$seconds" "$log" "$@"
