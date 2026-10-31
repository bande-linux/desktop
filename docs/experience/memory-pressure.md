# Running out of memory

Status: built on 2026-10-31 in `day/memory-pressure`. The research and the reasons are in
[`../decisions/0004-memory-pressure.md`](../decisions/0004-memory-pressure.md).

The owner's verdict after the dev VM froze twice under builds: "that shouldn't freeze and kill
the whole system". The bar is macOS: compress first, warn with a list of the heaviest apps,
and close an app rather than freeze.

| Now (built) | Later | Never |
| :--- | :--- | :--- |
| zram, `min(ram, 8192)`, zstd, recommended with `systemd-zram-generator` | Reopen action on the "was closed" notification that restores the app's windows | Our own userspace killer: systemd-oomd does it, outside the session it protects |
| systemd-oomd recommended; `app.slice` is the kill domain at 40 % full stall for 20 s | Terminal tabs in their own scopes (like VTE's `vte-spawn-*.scope`) so a runaway build closes one tab, not the whole Terminal | earlyoom: free-memory based and per process, not per app |
| Compositor and shell protected: `ManagedOOMPreference=avoid`, `OOMScoreAdjust=100` | Memory protection (`MemoryLow`) for the active session, once Debian has an active-seat allocator like uresourced | Pausing apps with SIGSTOP like macOS: Wayland shows them as hung and they keep their memory |
| Session services moved from `app.slice` to `session.slice` | Telling apps to drop caches at warning level (GMemoryMonitor already reads PSI; a portal signal for our own apps) | A swap-usage kill: Ubuntu turned it off after it killed responsive apps |
| Apps start at `oom_score_adj` 200, so the kernel's last-resort killer picks an app | `min_ttl_ms` thrashing guard, if oomd proves too slow on 8 GB machines | |
| "Your computer is running low on memory" window at a sustained full stall (5 % for 10 s), with the six heaviest apps, Force Quit with confirmation, Activity Monitor on the Memory tab | | |
| Notification naming the app (or background task) that systemd-oomd or the kernel closed | | |
