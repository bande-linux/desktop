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
start_dump() {
    local interface=$1 fifo line err; shift
    fifo=$(mktemp -u)
    mkfifo "$fifo"
    : > "$log.err"
    tcpdump -Z root -l -n --immediate-mode -i "$interface" -s 512 "$@" > "$log" 2> "$fifo" &
    dump=$!
    exec {err}< "$fifo"
    rm "$fifo"
    while IFS= read -r line <&"$err"; do
        printf '%s\n' "$line" >> "$log.err"
        [[ $line != *"listening on"* ]] || break
    done
    cat <&"$err" >> "$log.err" &
    exec {err}<&-
    [[ $line == *"listening on"* ]] || { cat "$log.err" >&2; return 1; }
}
stop_dump() {
    kill "$dump"
    wait
}
export -f report start_dump stop_dump
if [ -n "$live" ]; then
    coproc holder { exec unshare --net bash -c 'echo $$; exec sleep infinity'; }
    read -r target <&"${holder[0]}"
    ready=$(mktemp -u)
    mkfifo "$ready"
    slirp4netns --configure --mtu=65520 --ready-fd=3 "$target" tap0 > "$log.slirp" 2>&1 3> "$ready" &
    slirp=$!
    read -r -n 1 < "$ready" || { cat "$log.slirp" >&2; kill "$target"; exit 1; }
    rm "$ready"
    echo "nameserver 1.1.1.1" > /etc/resolv.conf
    set +e
    nsenter -t "$target" -n bash -c '
        log=$0 seconds=$1; shift 2
        ip link set lo up
        start_dump tap0 "port 53" || exit 1
        timeout "$seconds" "$@"
        status=$?
        stop_dump
        exit $status
    ' "$log" "$seconds" "$@"
    status=$?
    kill "$slirp" "$target" 2>/dev/null
    report
    exit "$status"
fi
exec unshare --net bash -c '
    seconds=$0 log=$1; shift
    ip link set lo up
    ip link add dummy0 type dummy
    ip addr add 10.0.2.100/24 dev dummy0
    ip link set dummy0 up
    ip route add default via 10.0.2.2
    echo "nameserver 10.0.2.3" > /etc/resolv.conf
    start_dump dummy0 -v || exit 1
    timeout "$seconds" "$@"
    stop_dump
    report
    ! grep -E " IP6? " "$log" | grep -vqE "ICMP6|igmp|ff02::"
' "$seconds" "$log" "$@"
