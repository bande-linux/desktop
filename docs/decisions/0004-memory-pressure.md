# Memory Pressure: Decision Record

Status: **decided** (October 2026). Scope: what happens when the computer runs out of memory,
from the kernel up to what the user sees. Everything here uses Debian packages only.

## Context

On 2026-10-30 the dev VM (16 GB RAM, a 3.4 GB swap partition, no zram, no userspace OOM
killer) froze twice under heavy builds. The kernel never logged an OOM kill: as long as it can
still reclaim page cache and push anonymous pages to swap, it counts that as progress, so it
keeps thrashing instead of killing. The machine was "alive" by the kernel's measure and useless
by the user's. Waiting for the kernel OOM killer is not a policy. A desktop needs its own.

## Decision

Four layers, each with one job, and nothing outside Debian.

| Layer | What | Owner |
| :--- | :--- | :--- |
| 1. More room | zram swap, `min(ram, 8192)`, zstd, ahead of any disk swap | `bande-desktop` (drop-in for `systemd-zram-generator`) |
| 2. Ask the user | at sustained memory stall, the shell's "running low on memory" window with the heaviest apps and Force Quit | `bande-shell` |
| 3. Act for the user | systemd-oomd kills the app with the most reclaim activity in `app.slice` | `bande-desktop` (drop-in for `app.slice`) and Debian's `systemd-oomd` |
| 4. Tell the user | a notification naming the app that was closed | `bande-shell` through `bande-xdg` |

### 1. zram

`/usr/lib/systemd/zram-generator.conf.d/50-bande.conf`:

```ini
[zram0]
zram-size = min(ram, 8192)
compression-algorithm = zstd
```

This is Fedora's rule (Debian's default is `min(ram / 2, 4096)` with the kernel's lz4). zram is
created with priority 100, so an existing swap partition stays as overflow. Debian's kernel has
`CONFIG_ZRAM_BACKEND_ZSTD=y`. With zram, oomd has time to act, as its manual recommends, and the
VM compresses before it writes. This is the macOS compressor model.

zram stacked over a disk swap partition is a known bad pairing: Fedora Asahi saw premature OOM
kills (Firefox) with it and moved swapfile systems to zswap. We add no disk swap. An admin who
does should prefer zswap (`zswap.enabled=1`) and turn zram off with an empty
`/etc/systemd/zram-generator.conf` and an empty `/etc/systemd/zram-generator.conf.d/50-bande.conf`.

We keep swappiness, `page-cluster` and the watermarks at the kernel defaults. Fedora ships zram
with defaults too. A sysctl file would also apply when an admin turns zram off and swaps to disk,
where `swappiness=180` is wrong.

### 2. Kill domain: `app.slice`, session protected

- `/usr/lib/systemd/user/app.slice.d/50-bande-oomd.conf`: `ManagedOOMMemoryPressure=kill`,
  `ManagedOOMMemoryPressureLimit=40%`, with Debian's default duration of 20 s. The systemd manual
  suggests about 40 % for latency-sensitive user cgroups. The candidates are the apps' scopes
  only. Debian's own 50 % kill on `user@.service` stays as the backstop.
- Every app already runs in its own `app-bande-<id>-<pid>.scope` in `app.slice`
  (`bande_xdg::spawn_detached`).
- The compositor (and with it the shell and every session client it starts) runs in
  `bande-compositor.service` in `session.slice`, with `ManagedOOMPreference=avoid` and
  `OOMScoreAdjust=100`. `avoid` is honoured because the candidate and the monitored
  `user@.service` cgroup have the same owner. `avoid` is not inherited, so it is set on the leaf
  unit itself.
- The session services (config, power, sound, tray, polkit agent, automount, portal) move from
  the default `app.slice` to `session.slice`. They are not apps, and no app-level kill should
  ever pick them.
- Apps start with `oom_score_adj` 200 (written in the child before `exec`), the same as user
  services. If the kernel OOM killer still has to act, it picks a big app and never the
  compositor at 100. Without this, apps would inherit the compositor's 100 and tiny session
  services at 200 would score above multi-GB apps.
- The few session-critical services Bande does not own get the same floor as the compositor,
  `OOMScoreAdjust=100`, through vendor drop-ins: `dbus.service` (the session bus),
  `pipewire.service`, `pipewire-pulse.service`, `wireplumber.service` and
  `xdg-desktop-portal.service`. On 2026-10-31 a global OOM on the mini killed `dbus.service` six
  times and PipeWire over and over, 59 kills of processes under 10 MB, while the eight 1.9 GB
  `rustc` at adj 0 that caused it lived: at 200, a 10 MB service scores about 196k pages
  against 119k for the build. At 100 it scores about 98k, below any process over about 1.5 GB.
  Once the session bus is gone, no session service or app reconnects.
- Swap kill stays off (`ManagedOOMSwap=auto`, Debian's default), for the reason Ubuntu found.
- No `oomd.conf` drop-in: Debian already sets `DefaultMemoryPressureDurationSec=20s`, and the
  per-unit limit above is the only threshold we change.

### 3. The low-memory window

The shell (one process, always running, fonts already loaded) opens a PSI trigger on
`/proc/pressure/memory`: `full 100000 2000000`, a full stall of at least 5 % in a 2 s window.
That is Activity Monitor's "Critical" level (full avg10 ≥ 5 %), so the window and the graph
agree. It is the system-wide file, because thrashing hurts the whole machine whoever causes it.
`/proc/pressure/*` is mode 0666 on Debian 13 (Linux 6.12), and a 2 s window is allowed for
unprivileged users. The fd sits in the shell's `poll()` set. Idle cost is zero: no timer, no
reads, and the kernel only arms the trigger while memory stalls.

Debounce (`bande_status::memory::Streak`): each trigger event is one stalled window. Events
less than 5 s apart form a streak. When a streak lasts 10 s, the window opens. Closing it snoozes
until memory has been calm for 60 s, so it never nags during one bad build.

The window (in the shell process, not a new one, because starting a process when memory is
short is exactly what must be avoided) is a toplevel kept on top:

- Activity Monitor's icon as the alert disc, the heading "Your computer is running low on
  memory", and a line telling the user to quit apps they are not using.
- The six heaviest apps, read from each app scope's `memory.current + memory.swap.current` and
  grouped by app (two windows of one app add up), with icon, name and size. The list refreshes
  every 2 s while the window is open, and only then.
- **Force Quit** asks first ("Force quit Terminal? Any unsaved changes in it are lost.") and
  then calls systemd's `KillUnit(scope, "all", SIGKILL)`, which kills the app's whole cgroup.
- **Activity Monitor** opens it on the Memory tab (`bande-monitor --memory`), where the
  pressure graph is; the button is hidden when Activity Monitor is not installed.
- **Close** (Escape) dismisses it.

### 4. The notification

`bande_xdg::follow_oom_kills` subscribes to the user manager (`Subscribe`, because systemd
only sends unit signals while someone is subscribed) and watches `PropertiesChanged` on
`/org/freedesktop/systemd1/unit/*`. Since systemd 248 both a kernel OOM kill and a systemd-oomd
kill set a scope's or service's `Result` to `oom-kill`. The unit name is decoded from the bus
path, the app from the unit name (the same decoder Activity Monitor uses, now shared), and the
notification says "Terminal was closed because memory ran low". Clicking it opens the app again,
as the crash notice does. Units that are not apps (a build in `run-*.scope`, a service) get "A
background task was stopped because memory ran low", naming the unit.

## Rejected

- **earlyoom.** It is in Debian, but it acts on free memory and swap, not on stall time, and it
  kills processes, not cgroups. systemd-oomd is the systemd-native, cgroup-aware choice that
  Fedora, Ubuntu and Debian's own defaults build on.
- **Our own killer in the shell.** That would duplicate oomd and run in the session it must
  protect.
- **`min_ttl_ms` by default.** The MGLRU knob makes the kernel kill rather than thrash, but the
  kernel OOM killer picks by size, not by app. Behind oomd it is a second, cruder killer.
  Revisit if oomd turns out too slow on low-RAM machines.
- **A system-wide `MemoryLow` for `user@.service`** (what uresourced does dynamically). A static
  value would protect every logged-in user, including inactive ones. Without uresourced in
  Debian, there is no active-seat tracking to do it right.
- **Pausing apps (SIGSTOP) like macOS.** Wayland clients that stop answering pings are shown as
  hung, and a stopped app holds its memory. The user chooses in the window instead.

## Sources

1. Apple, Activity Monitor User Guide, "View memory usage in Activity Monitor on Mac",
   https://support.apple.com/guide/activity-monitor/view-memory-usage-actmntr1004/mac
2. Apple Developer, "Responding to low-memory warnings",
   https://developer.apple.com/documentation/xcode/responding-to-low-memory-warnings
3. XNU, `doc/vm/memorystatus_notify.md` (pressure levels and dispatch flags).
4. Apple Community threads on "Your system has run out of application memory", e.g.
   https://discussions.apple.com/thread/7520039
5. Microsoft Learn, "Introduction to the page file",
   https://learn.microsoft.com/windows/client-management/introduction-page-file
6. Microsoft Learn, `CreateMemoryResourceNotification`,
   https://learn.microsoft.com/en-us/windows/win32/api/memoryapi/nf-memoryapi-creatememoryresourcenotification
7. Microsoft, Resource Exhaustion Detector and Resolver events (Windows Server 2008 R2 docs),
   https://learn.microsoft.com/en-us/previous-versions/windows/it-pro/windows-server-2008-r2-and-2008/cc774657(v=ws.10)
8. Fedora, "Changes/EnableSystemdOomd", https://fedoraproject.org/wiki/Changes/EnableSystemdOomd
9. GLib, `gio/gmemorymonitorpsi.c`,
   https://gitlab.gnome.org/GNOME/glib/-/blob/main/gio/gmemorymonitorpsi.c
10. KDE, plasma-workspace merge request 4204 (oom-notifier),
    https://invent.kde.org/plasma/plasma-workspace/-/merge_requests/4204
11. Launchpad bug 1972159, "systemd-oomd frequently kills firefox and visual studio code",
    and https://www.phoronix.com/news/Ubuntu-Drops-Swap-Kill
12. Android, "Low memory killer daemon", https://source.android.com/docs/core/perf/lmkd
13. Linux kernel, "PSI - Pressure Stall Information", https://docs.kernel.org/accounting/psi.html
14. Linux kernel, "Multi-Gen LRU", https://docs.kernel.org/admin-guide/mm/multigen_lru.html
15. systemd 257 manuals: `systemd.resource-control(5)` (ManagedOOM*), `systemd-oomd.service(8)`,
    `oomd.conf(5)`; systemd v257 `src/core/cgroup.c` (`unit_check_oomd_kill`) and
    `src/core/scope.c` (`SCOPE_FAILURE_OOM_KILL`); `zram-generator.conf(5)`.
