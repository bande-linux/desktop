# Performance

How to measure memory, CPU and wake-ups of the session, and where the numbers stand. The frame
time targets are in [`../decisions/0001-rendering.md`](../decisions/0001-rendering.md).

## How to measure

Measure when a change is about memory, CPU or wake-ups: before and after, with release builds of
both sides from the same worktree and its own target directory, under the build lock
(`flock ~/bande-linux/kbuild.lock ~/bande-linux/bin/gentle …` on the build machine). Live numbers
on the mini are read from `/proc` only; nothing restarts the owner's session. The tools are in
`crates/testing/bench` and run on a headless compositor inside a private bus, so they never
reach a running session.

- **The whole session at idle:** `bande-idle --binaries <dir>` runs the compositor, the shell,
  the services and the apps found in `<dir>` with a private home folder and reports RSS and
  wake-ups per process with windows open, minimised and locked.
- **One client's memory and CPU:** `bande-footprint` starts a client as a shell client on a
  2928×1818 output at 1.5×, lets it settle (`--settle <seconds>`), optionally moves the pointer
  over it, and prints RSS + swap, Vulkan device memory, threads and CPU per pointer move. For an
  exact number read `Anonymous` from `/proc/<pid>/smaps_rollup`. `BANDE_RENDERER=gpu` or `=cpu`
  picks the path.

  ```sh
  cargo build --release -p bande-bench -p bande-shell
  dbus-run-session -- target/release/bande-footprint --hover 2864,70,2600,900 target/release/bande-shell
  ```
- **Frame times:** `bande-session-bench` runs the compositor in-process with the shell, Files and
  Terminal (`bande-terminal` from bande-linux/apps in `target/release` or on `PATH`) and replays
  typical actions (pointer over Files, dragging a window, typing, scrolling, the launcher, the
  overview, a context menu, banners); it prints frame time p50/p99, CPU per frame and pixels
  repainted. `bande-toolkit-bench` measures the toolkit sample (`crates/testing/sample`).
- **Files at 100 000 items:** `bande-files-bench` drives `FilesView` in-process over a folder of
  100 000 files and prints UI-thread time per step.
- **Wake-ups by hand:** sum `voluntary_ctxt_switches` and `nonvoluntary_ctxt_switches` over
  `/proc/<pid>/task/*/status` twice and divide by the interval; the threads' `comm` shows which
  one wakes.
- **Size:** `cargo build --profile dist -p <package>` (the packages' profile: fat LTO, one
  codegen unit, `panic = "abort"`, stripped); crate counts with
  `cargo tree -e normal -p <package> --prefix none | sort -u | wc -l`.

lavapipe draws on the CPU, so its GPU memory shows up as process memory; on Apple Silicon GPU
memory is system memory too, but the real numbers need the mini's GPU.

## Where it stands

Measured 2026-11-01. Idle at the login window on the mini (installed dist packages, one
2560×1440 display): the greeter's compositor 50 MB RSS and the greeter 40 MB, every user service
2–4 MB and asleep in `epoll` with no timer; the compositor wakes only for its systemd watchdog.
Under `bande-footprint` on the CPU path the shell holds about 78 MiB anonymous memory and the
greeter about 26 MiB. Hidden windows drop their renderer after 30 s, decoded wallpapers are freed
once uploaded, and animations, sampling and timers run only while something on screen needs
them, so an idle process should not wake at all.

## What wakes when idle

Everything else waits on an event: Wayland, D-Bus signals with match rules narrowed to what is
shown (UPower only for its display device), inotify, pidfds for child processes, PipeWire and
eventfds. These are the only timers left; each runs only while its condition holds.

| What | When | Why |
| :--- | :--- | :--- |
| Compositor | every 10 s | systemd watchdog (`WatchdogSec=20s`) |
| Night light | at the next scheduled change | real-time deadline; clock and time zone changes are events |
| Bar clock, Calendar's today | at each minute boundary | the display unit |
| Countdowns (log out, screenshot timer), Now Playing | at each second boundary while shown | the display unit |
| Notification Center | when an age label next changes, while open | the display unit (minutes, hours, midnight) |
| Banners, alarms, calendar alerts | at their deadline | real deadlines |
| Activity Monitor, the low memory window | every 2 s while a window is on screen | `/proc` has no change events |
| Settings' Sound pane | 20 times a second while on screen | the input level meter |
| Disk space (automount) | every 10 min | free space has no kernel event |
| Files | once a day | emptying old Trash items |
| Mail | IDLE renewed every 25 min; full sync every 15 min; polling every 5 min only without IDLE | servers drop IDLE after 30 min and IDLE watches only the inbox |
| Calendar | CalDAV every 5 min, subscriptions hourly | CalDAV has no push |

Installed dist binaries are 14–21 MB for each GUI program (`bande-shell` 21, Settings 19, Files
17, the compositor 14) and about 2–3 MB for each service. Every GUI binary carries its own copy of
the toolkit, Vello, wgpu, Parley and zbus, including about 2.9 MB of ICU segmentation data for
Thai, Lao, Khmer, Burmese and CJK text; sharing them (a multicall binary, or ICU data loaded at
run time) is an open decision.
