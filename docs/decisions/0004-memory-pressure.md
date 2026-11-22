# Memory Pressure: Decision Record

Status: **decided** (October 2026). Scope: what happens when the computer runs out of memory,
from the kernel up to what the user sees. Everything here uses Debian packages only.

## Context

Under heavy memory use the kernel rarely kills: as long as it can still reclaim page cache and
push anonymous pages to swap, it counts that as progress, so it keeps thrashing instead of
killing. The computer is "alive" by the kernel's measure and useless by the user's. Waiting for
the kernel OOM killer is not a policy. A desktop needs its own.

## Decision

Three layers, each with one job, and nothing outside Debian. Swap, zram and the system's
memory settings stay the system's own.

| Layer | What | Owner |
| :--- | :--- | :--- |
| 1. Ask the user | at sustained memory stall, the shell's "running low on memory" window with the heaviest apps and Force Quit | `bande-shell` |
| 2. Act for the user | systemd-oomd kills the app with the most reclaim activity in `app.slice` | `bande-session` (runtime properties of `app.slice`) and Debian's `systemd-oomd` |
| 3. Tell the user | a notification naming the app that was closed | `bande-shell` through `bande-xdg` |

### 1. Kill domain: `app.slice`

- `bande-session` sets `ManagedOOMMemoryPressure=kill` and `ManagedOOMMemoryPressureLimit=40%`
  on `app.slice` with `systemctl --user set-property --runtime` as the session starts, so other
  desktops' apps keep systemd's defaults. Debian's default duration of 20 s stays. The systemd manual
  suggests about 40 % for latency-sensitive user cgroups [3]. The candidates are the apps' scopes
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
  `xdg-desktop-portal.service`. At 200, a 10 MB service would score about 196k pages, above a
  1.9 GB process at adj 0 (about 119k), so a global OOM would kill small session services over
  and over while the process that caused it lived. At 100 it scores about 98k, below any process
  over about 1.5 GB. Once the session bus is gone, no session service or app reconnects.
- The session bus, PipeWire, WirePlumber and the portal also get `ManagedOOMPreference=avoid`,
  so Debian's backstop on `user@.service` passes them over while an app remains.
- Swap kill stays off (`ManagedOOMSwap=auto`, Debian's default): killing on swap use closes
  large apps that are only idle, not thrashing.
- No `oomd.conf` drop-in: Debian already sets `DefaultMemoryPressureDurationSec=20s`, and the
  per-unit limit above is the only threshold we change.

### 2. The low-memory window

The shell (one process, always running, fonts already loaded) opens a PSI trigger on
`/proc/pressure/memory` [1]: `full 100000 2000000`, a full stall of at least 5 % in a 2 s window.
That is System Monitor's "Critical" level (full avg10 ≥ 5 %), so the window and the graph
agree. It is the system-wide file, because thrashing hurts the whole machine whoever causes it.
`/proc/pressure/*` is mode 0666 on Debian 14 (forky), and a 2 s window is allowed for
unprivileged users. The fd sits in the shell's `poll()` set. Idle cost is zero: no timer, no
reads, and the kernel only arms the trigger while memory stalls.

Debounce (`bande_status::memory::Streak`): each trigger event is one stalled window. Events
less than 5 s apart form a streak. When a streak lasts 10 s, the window opens. Closing it snoozes
until memory has been calm for 60 s, so it never nags during one bad build.

The window (in the shell process, not a new one, because starting a process when memory is
short is exactly what must be avoided) is a toplevel kept on top:

- System Monitor's icon as the alert disc, the heading "Your computer is running low on
  memory", and a line telling the user to quit apps they are not using.
- The six heaviest apps, read from each app scope's `memory.current + memory.swap.current` and
  grouped by app (two windows of one app add up), with icon, name and size. The list refreshes
  every 2 s while the window is open, and only then.
- **Force Quit** asks first ("Force quit Terminal? Any unsaved changes in it are lost.") and
  then calls systemd's `KillUnit(scope, "all", SIGKILL)`, which kills the app's whole cgroup.
- **System Monitor** opens it on the Memory tab (`bande-monitor --memory`), where the
  pressure graph is; the button is hidden when System Monitor is not installed.
- **Close** (Escape) dismisses it.

### 3. The notification

`bande_xdg::follow_oom_kills` subscribes to the user manager (`Subscribe`, because systemd
only sends unit signals while someone is subscribed) and watches `PropertiesChanged` on
`/org/freedesktop/systemd1/unit/*`. Since systemd 248 both a kernel OOM kill and a systemd-oomd
kill set a scope's or service's `Result` to `oom-kill` [3]. The unit name is decoded from the bus
path, the app from the unit name (the decoder System Monitor uses too), and the
notification says "Terminal was closed because memory ran low". Clicking it opens the app again,
as the crash notice does. Units that are not apps (a build in `run-*.scope`, a service) get "A
background task was stopped because memory ran low", naming the unit.

## Rejected

- **earlyoom.** It is in Debian, but it acts on free memory and swap, not on stall time, and it
  kills processes, not cgroups. systemd-oomd is the systemd-native, cgroup-aware choice that
  Debian's own defaults build on.
- **Our own killer in the shell.** That would duplicate oomd and run in the session it must
  protect.
- **`min_ttl_ms` by default.** The MGLRU knob [2] makes the kernel kill rather than thrash, but the
  kernel OOM killer picks by size, not by app. Behind oomd it is a second, cruder killer.
- **Pausing apps (SIGSTOP).** Wayland clients that stop answering pings are shown as
  hung, and a stopped app holds its memory. The user chooses in the window instead.

## Sources

1. Linux kernel, "PSI - Pressure Stall Information", https://docs.kernel.org/accounting/psi.html
2. Linux kernel, "Multi-Gen LRU", https://docs.kernel.org/admin-guide/mm/multigen_lru.html
3. systemd 257 manuals: `systemd.resource-control(5)` (ManagedOOM*), `systemd-oomd.service(8)`,
   `oomd.conf(5)`; systemd v257 `src/core/cgroup.c` (`unit_check_oomd_kill`) and
   `src/core/scope.c` (`SCOPE_FAILURE_OOM_KILL`).
