#!/bin/bash
# mem-peak.sh <log> <command...>: runs the command and writes to <log> the peak anonymous memory of
# its cgroup (real need, without disk cache), sampled every 2 s, with the cgroup's memory limit.
log=$1; shift
cgroup=/sys/fs/cgroup$(sed -n 's/^0:://p' /proc/self/cgroup)
(
    peak=0
    while :; do
        anon=$(awk '$1 == "anon" { print $2 }' "$cgroup/memory.stat" 2>/dev/null)
        if [ "${anon:-0}" -gt "$peak" ]; then
            peak=$anon
            echo "peak $((peak / 1048576)) MiB at $(date +%T), limit $(cat "$cgroup/memory.max" 2>/dev/null), $(ps -eo rss=,comm= --sort=-rss | head -1)" > "$log"
        fi
        sleep 2
    done
) &
sampler=$!
"$@"
status=$?
kill "$sampler" 2>/dev/null
exit "$status"
