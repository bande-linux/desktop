# Memory Pressure: Decision Record

Status: **decided** (October 2026). Scope: what happens when the computer runs out of memory,
from the kernel up to what the user sees. Everything here uses Debian packages only.

## Context

Under heavy memory use the kernel rarely kills: as long as it can still reclaim page cache and
push anonymous pages to swap, it counts that as progress, so it keeps thrashing instead of
killing. The computer is "alive" by the kernel's measure and useless by the user's. Waiting for
the kernel OOM killer is not a policy. A desktop needs its own.

## Decision

Four layers, each with one job, and nothing outside Debian. Swap, zram and the system's
memory settings stay the system's own.

| Layer | What | Owner |
| :--- | :--- | :--- |
| 0. Keep and slow | memory kept for the desktop; `app.slice` is slowed and swapped first, while a reserve stays free | `bande-login`, `bande-session` (runtime properties) |
| 1. Ask the user | at sustained memory stall, the shell's "running low on memory" window with the heaviest apps and Force Quit | `bande-shell` |
| 2. Act for the user | systemd-oomd kills the app with the most reclaim activity in `app.slice` | `bande-session` (runtime properties of `app.slice`) and Debian's `systemd-oomd` |
| 3. Tell the user | a notification naming the app that was closed | `bande-shell` through `bande-xdg` |

### 0. Memory for the desktop

- While a person's Bande session runs, `bande-login` sets `MemoryMin=` at runtime on `user.slice`,
  `user-UID.slice` and `user@UID.service`, and `bande-session` on `session.slice`: a tenth of the
  memory, at most 512 MiB. The kernel honours a guarantee only as far as every level above grants
  it; `memory_recursiveprot` shares it among `session.slice`'s units by use. `MemoryMin=` and not
  `MemoryLow=`: when nothing else can be reclaimed, the kernel breaks `MemoryLow=` and the desktop
  stalls. A level where the administrator set a value is left alone. `bande-login` sets
  the values back to 0 when the session ends, `user.slice` only when no other Bande session is left.
- `MemoryMin=` does not cover the compositor's code: file pages belong to the control group that read
  them first, the login window's compositor (the same program), which is gone by then. Reading them
  again took up to 0.3 s each while something swapped. The compositor of a person's session therefore
  locks the read-only file mappings of itself and its libraries on fault (`mlock2`,
  `MLOCK_ONFAULT`: only pages that are used, and stay used), itself first, as far as its
  `LimitMEMLOCK=256M` goes. The user manager may only grant that because `bande-login` raises that
  process's hard limit while the session runs; other units keep the 8 MiB default. Without it the
  compositor locks what fits in the limit it has.
- `app.slice` gets `MemoryHigh=` all memory but a fifth of it, at most 3 GiB: the desktop's guarantee, the
  system and free memory. Apps together above
  that are reclaimed and slowed, so the desktop and the system still find free memory at once.
- Each app's scope gets `MemoryMin=128M` and `MemoryLow=512M`. When `app.slice` is reclaimed at its
  limit that keeps the other apps' pages, so a build in a Terminal tab does not swap out the windows
  on screen; a tab's own scope has neither. systemd-oomd picks the scope the kernel scans most: with
  `MemoryLow=` alone it picked small apps the kernel scanned when the build's pages were hard to
  free. These guarantees hold only inside `app.slice`, which has none itself, so they take nothing
  from the rest of the system.
- `bande-session` removes the runtime properties it set when the session ends.

### 1. Kill domain: `app.slice`

- `bande-session` sets `ManagedOOMMemoryPressure=kill` and `ManagedOOMMemoryPressureLimit=40%`
  on `app.slice` with `systemctl --user set-property --runtime` as the session starts, so other
  desktops' apps keep systemd's defaults, with `ManagedOOMMemoryPressureDurationSec=10s` there
  instead of Debian's 20 s. The systemd manual
  suggests about 40 % for latency-sensitive user cgroups [3]. The candidates are the apps' scopes
  only. Debian's own 50 % kill on `user@.service` stays as the backstop.
- Every app already runs in its own `app-bande-<id>-<pid>.scope` in `app.slice`
  (`bande_xdg::spawn_detached`), and what runs in a Terminal tab in
  `app-bande-org.bande.Terminal-tab<pid>.scope` (`place_tab_in_scope`), so oomd and Force Quit end
  that tab alone. The shell of a new tab waits, stopped, until it is in its scope (2 s at most), so
  nothing it starts is left behind. The tab scope has `OOMPolicy=continue`: a kernel kill of one
  process leaves the tab's shell. A tab ended as a whole stays open and says so until Return.
- The compositor (and with it the shell and every session client it starts) runs in
  `bande-compositor.service` in `session.slice`, with `ManagedOOMPreference=avoid` and
  `OOMScoreAdjust=100`, the lowest a user unit can set. `avoid` is honoured because the candidate
  and the monitored `user@.service` cgroup have the same owner. `avoid` is not inherited, so it is
  set on the leaf unit itself. The session clients (shell, lock screen, screenshot tool,
  clipboard daemon) start at 200 like the services (`bande_xdg::adopt_service_oom_score` before
  `exec`), so the kernel kills one of them before the compositor, which then starts it again:
  the unit has `OOMPolicy=continue`, since systemd's default `stop` would end the whole session
  when the kernel kills the shell. The unit sets no `MemoryMin=` itself: `session.slice` has it.
- The session services (config, power, sound, tray, polkit agent, automount, portal) move from
  the default `app.slice` to `session.slice`. They are not apps, and no app-level kill should
  ever pick them.
- Apps start with `oom_score_adj` 200 (written in the child before `exec`), the same as user
  services. If the kernel OOM killer still has to act, it picks a big app and never the
  compositor at 100. Without this, apps would inherit the compositor's 100 and tiny session
  services at 200 would score above multi-GB apps.
- The session-critical services Bande does not own, `dbus.service` (the session bus),
  `pipewire.service`, `pipewire-pulse.service`, `wireplumber.service` and
  `xdg-desktop-portal.service`, get `ManagedOOMPreference=avoid` from `bande-session` at start
  (`systemctl --user set-property --runtime`), so Debian's backstop on `user@.service` passes
  them over while an app remains. Their units stay as their packages ship them, so other
  desktops see no change. They keep systemd's `oom_score_adj` of 200, the same as apps, so the
  kernel's OOM killer still picks a large app before a small service. The exception is a
  runaway process at adj 0 (a system service): above about 1.5 GB it scores below a 10 MB
  service at 200. A lower `OOMScoreAdjust` would cover that, but it applies only at exec,
  through a drop-in on another package's unit for every desktop, so Bande leaves it.
- Swap kill stays off (`ManagedOOMSwap=auto`, Debian's default): killing on swap use closes
  large apps that are only idle, not thrashing.
- No `oomd.conf` drop-in: Debian already sets `DefaultMemoryPressureDurationSec=20s`, and the
  per-unit limit above is the only threshold we change.

### 2. The low-memory window

The shell (one process, always running, fonts already loaded) opens a PSI trigger on `app.slice`'s
`memory.pressure` [1] (the whole computer's `/proc/pressure/memory` when it cannot): `full 100000
2000000`, a full stall of at least 5 % in a 2 s window.
That is System Monitor's "Critical" level (full avg10 ≥ 5 %), so the window and the graph
agree. It is `app.slice`'s file, because at `MemoryHigh=` the apps stall and the rest does not. The
user manager owns it, and a 2 s window is allowed for unprivileged users. The fd sits in the shell's `poll()` set. Idle cost is zero: no timer, no
reads, and the kernel only arms the trigger while memory stalls.

Debounce (`bande_status::memory::Streak`): each trigger event is one stalled window. Events
less than 5 s apart form a streak. When a streak lasts 4 s and memory and swap are nearly used up or
the apps are at their limit, the window opens, before systemd-oomd acts. Closing it snoozes
until memory has been calm for 60 s, so it never nags during one bad build.

The window (in the shell process, not a new one, because starting a process when memory is
short is exactly what must be avoided) is a toplevel kept on top:

- System Monitor's icon as the alert disc, the heading "Your computer is running low on
  memory", and a line telling the user to quit apps they are not using.
- The six heaviest apps, read from each app scope's `memory.current + memory.swap.current` and
  grouped by app (two windows of one app add up; a Terminal tab is its own row, "cargo in Terminal"), with icon, name and size. The list refreshes
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
notification says "Files was closed because memory ran low" ("A tab in Terminal was stopped
because memory ran low" for a tab's scope). Clicking it opens the app again,
as the crash notice does. Units that are not apps (a build in `run-*.scope`, a service) get "A
background task was stopped because memory ran low", naming the unit.

## Rejected

- **earlyoom.** It is in Debian, but it acts on free memory and swap, not on stall time, and it
  kills processes, not cgroups. systemd-oomd is the systemd-native, cgroup-aware choice that
  Debian's own defaults build on.
- **Locking all of the compositor's memory** (`mlockall`). `MCL_FUTURE` makes every later mapping
  above `RLIMIT_MEMLOCK` fail, client buffers included, and `MemoryMin=` already covers the memory
  the compositor allocates.
- **Real-time drawing.** See [session.md](../architecture/session.md): 200 ms of computing ends the
  process. Short slices keep the pointer within the target.
- **Changing weights with the focus.** Each scope already has an equal share; a change on every
  focus change would write a drop-in and wake the service manager each time.
- **I/O weights.** The `io` controller is not delegated to the user manager, and enabling it changes
  the whole system; a disk writer alone does not stall the pointer.
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
