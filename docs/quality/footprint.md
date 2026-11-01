# Footprint

Memory, CPU and wake-ups of the Bande processes at idle, before the first release. How to
measure is in [`../architecture/performance.md`](../architecture/performance.md); the plan for
what is left is [`NOTES/plans/footprint.md`](../../NOTES/plans/footprint.md).

## 2026-11-01: live baseline on the mini

The M2 mini (15 GB, one 2560×1440 HDMI display), installed dist packages from `main` at
`6b95e86`, read from `/proc` only. Nobody was signed in: greetd showed the greeter inside
`bande-compositor --greeter`, and the user services of uid 1000 were running without a
session. PSS needs root for the greeter's processes, so RSS is split from
`/proc/<pid>/status`. Wake-ups are the sum of every thread's voluntary and involuntary context
switches over 60 s; CPU is `utime + stime` over the process's life.

| Process | RSS MB | anon | file | Threads | Wake-ups/s (60 s) | Wake-ups/s (life) | CPU s / life |
| :--- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| bande-compositor --greeter | 49.5 | 12.5 | 37.0 | 8 | 0.08 | 0.15 | 0.58 / 3.8 h |
| bande-greeter | 39.8 | 28.3 | 11.5 | 10 | 6.1 | 12.8 | 4.21 / 3.8 h |
| bande-config (greeter user) | 1.6 | 0.7 | 0.9 | 5 | 0 | 0.01 | 0 |
| xdg-desktop-portal-bande (greeter user) | 2.4 | 1.1 | 1.3 | 10 | 0 | 0.02 | 0 |
| bande-config | 3.7 | 0.8 | 2.9 | 5 | 0 | 0.7 (start-up) | 0.03 |
| bande-tray | 3.4 | 0.7 | 2.8 | 4 | 0 | 0.6 (start-up) | 0.03 |
| bande-notify | 3.1 | 0.7 | 2.4 | 7 | 0 | 0.16 | 0.03 |
| xdg-desktop-portal-bande | 4.4 | 1.1 | 3.3 | 10 | 0 | 0.08 | 0 |
| **Total** | **107.9** | | | **59** | **6.2** | | |

- **The compositor** wakes only for its systemd watchdog (`WatchdogSec=20s`, a ping every
  10 s).
- **The greeter** woke in bursts: 180–275 context switches each time NetworkManager announced
  a new `Strength` for the one access point in range (every 10–20 s; 5 signals and 1088
  switches in 88 s). `bande_status::devices` re-read all of Wi-Fi for every signal: every
  device, every access point, every saved connection's settings, connectivity. The bar and the
  lock screen run the same follower. Fixed in `perf/wifi-strength` (below).
- **The greeter's 28 MB of anonymous memory** holds two pictures: the blurred backdrop
  (decoded at a reduced size) and the sharp wallpaper at the window's size (14.7 MB of RGBA
  at 2560×1440, about half), kept for the whole wait only to draw the unveil after sign-in.
- **The user services** sleep in `epoll` with no timer; their life averages are start-up.

## Binaries

Installed dist builds (`[profile.dist]`: fat LTO, `codegen-units = 1`, `panic = "abort"`,
stripped). Sizes in MB:

| Binary | MB | Binary | MB |
| :--- | ---: | :--- | ---: |
| bande-shell | 21.0 | bande-term | 14.3 |
| bande-settings | 18.9 | bande-compositor | 14.1 |
| bande-files | 16.8 | bande-installer | 13.0 |
| bande-lock | 15.3 | bande-software | 12.8 |
| bande-greeter | 15.3 | bande-polkit-agent | 12.8 |
| bande-monitor | 14.9 | xdg-desktop-portal-bande | 4.2 |
| bande-screenshot | 14.5 | bande-power, -config, -automount, -notify, -sound, -tray, -clipboard | 1.8–2.8 |

`bande-shell` is 14.6 MB of `.text` and 4.4 MB of `.rodata`. Every GUI binary carries its own
copy of the toolkit, vello, wgpu, parley and zbus, so their code pages are not shared between
processes (plan milestone 7).

## 2026-11-01: the session under `bande-idle`

The logged-in session could not be measured live (nobody was signed in). `bande-idle`
(`crates/testing/bench`) ran the installed dist binaries of `6b95e86` instead (`--binaries`
pointing at `/usr/bin`, with `xdg-desktop-portal-bande` linked in from `/usr/libexec`) on its
headless 2928×1818 compositor at 1.5×, inside the test bus, with the real system bus
(NetworkManager, UPower, BlueZ) and no PipeWire. `bande-power` was replaced by a sleeping
stand-in so that no idle policy could reach logind. lavapipe draws everything, so the
compositor's RSS includes its GPU memory and every client draws on the CPU path; on the mini's
GPU both are lower. 60 s per case after 20 s of settling, empty `HOME`:

| Process | RSS MiB | Wake-ups/s: windows open | minimised | locked | Waking threads |
| :--- | ---: | ---: | ---: | ---: | :--- |
| bande-compositor (lavapipe) | 274–347 | 8.55 | 0.52 | 0.28 | the bench's own loop, lavapipe |
| bande-shell | 171 | 16.60 | 6.89 | 10.40 | zbus, async-io, main: Wi-Fi re-reads |
| bande-settings | 87.6 | 31.19 | 13.99 | 20.94 | zbus, async-io, `network`: Wi-Fi re-reads |
| bande-lock | 89.4 | | | 9.70 | zbus, async-io, main: Wi-Fi re-reads |
| bande-monitor | 80.8 | 4.60 | 0.58 | 0.55 | its sampler (every 2 s, slower when hidden) |
| bande-files | 79.4 | 0 | 0 | 0 | |
| bande-term | 72.2 | 0 | 0 | 0 | |
| bande-screenshot | 66.8 | 0 | 0 | 0 | |
| xdg-desktop-portal-bande | 6.0 | 0.07 | 0 | 0 | |
| bande-sound | 5.8 | 0 | 0 | 0 | |
| bande-automount | 5.0 | 0.07 | 0 | 0 | |
| bande-config | 4.4 | 0 | 0 | 0 | |
| bande-notify | 4.1 | 0 | 0 | 0 | |
| bande-clipboard | 3.9 | 0 | 0 | 0 | |
| bande-tray | 3.7 | 0 | 0 | 0 | |

- **Every Wi-Fi follower re-read NetworkManager on each signal-strength change**: the bar (in
  the shell), the lock screen, the greeter and Settings. With one access point in range that
  is a signal every 10–30 s and 180–300 context switches per re-read.
- **With the real `HOME`** (`/home/mikael`, 180 000 indexable entries in 12 000 folders, almost
  all of them `~/bande-linux/repos`) the shell's file index holds about 20 MB more (171 → 191
  MiB) and its `folder-watch` and `file-index` threads followed the other agents' builds
  writing into the home (24–63 wake-ups/s). That is the index doing its job, but its memory
  grows with the home; see the plan.
- **Minimising** needed Super+H since the Super→Ctrl change; `bande-idle` still pressed
  Super+M and failed at that step. Fixed.

## 2026-11-01: Wi-Fi strength applied in place

A `PropertiesChanged` from an access point carrying only `Strength` (and `LastSeen`) now
updates that network's strength in the shown list (`bande_network::set_strength`, which
re-sorts), and one carrying only `LastSeen` is ignored; any other signal still re-reads.
`bande_status::devices` (bar, lock screen, greeter) and Settings share
`bande_network::strength_signal`. The greeter also follows devices on its one system bus
connection instead of a second one.

`follow_devices` on the real system bus, the old and new code side by side for the same
600 s (a throwaway harness, release build, its own poll adds 0.2/s): NetworkManager sent 23
`Strength` signals; both delivered the same 23 updates.

| | Context switches in 600 s | per second |
| :--- | ---: | ---: |
| before (`main`) | 6345 | 10.6 |
| after | 436 | 0.73 |

The whole session, `bande-idle` with the installed dist binaries (before) and with
`bande-shell`, `bande-settings` and `bande-lock` from this branch (after, release builds without
LTO, hence 4–6 MiB more RSS), both running at the same time so that they saw the same
NetworkManager signals, 120 s per case, wake-ups per second:

| Process | windows open: before | after | minimised: before | after | locked: before | after |
| :--- | ---: | ---: | ---: | ---: | ---: | ---: |
| bande-shell | 0.15 | 0.19 | 19.08 | 0.79 | 8.56 | 0.31 |
| bande-lock | | | | | 8.14 | 0.25 |
| bande-settings | 0.79 | 0.77 | 36.88 | 5.30 | 15.43 | 2.43 |

No strength signal arrived during the first case. What Settings still does is its BlueZ
follower: it re-reads every adapter and device on any BlueZ signal (its `bluetooth` thread woke
111 times in the 120 s), the same pattern as Wi-Fi was.

## 2026-11-01: icon lookups cached

`bande_xdg::find_icon` read and parsed every theme's `index.theme` on each call and, when the
size missed, called `stat` on every candidate in every theme folder for every base folder and
extension. It now keeps the parsed themes and each folder's file names, checked again by
modification time at most every 2 s (ported from `wip/userland-efficiency` f6ce554c). The
installed apps' icons at 48 px, looked up three times in one process (17 icons, 51 lookups,
release build, warm page cache, `strace -f -c`):

| | system calls | of them `statx` | time |
| :--- | ---: | ---: | ---: |
| before | 34 742 | 34 022 | 41–71 ms |
| after | 5 765 | 2 168 | 11–20 ms |

The cache holds the names of the folders it has listed: about 1 500 names for Adwaita, hicolor
and Bande together.
