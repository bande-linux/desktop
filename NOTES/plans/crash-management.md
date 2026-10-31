# Crash management

## Summary

The journal shows how running out of memory took the whole session down. On 2026-10-31,
07:33-07:37, a build filled RAM (no swap, no PSI, no oomd). The kernel OOM killer then killed 59
small session processes (the session bus six times, PipeWire, WirePlumber, portals) and left
the eight 1.9 GB `rustc` processes that caused it alive. The cause is `oom_score_adj`. Every
service in the user manager runs at 200, which adds about 3 GB of "badness" to a 10 MB
process, while the build ran at 0. With the session bus gone, every desktop service and app
loses D-Bus, and the desktop looks frozen. Memory-pressure decision 0004 already has the
right design, but none of it runs on this kernel (no PSI, no zram).

Freezes with no OOM leave no evidence. The kernel has no lockup detectors, the hardware
watchdog is unused, and `kernel.panic` is 0. A hung compositor is never noticed either: it
runs as `Type=notify` with no `WatchdogSec`.

Fix the layers bottom-up: kernel (PSI, zram, detectors), image (oomd, zram, hardware watchdog,
reboot on panic), desktop (protect the session's few critical services, a compositor
watchdog), then test at the machine.

## Findings

- **The 2026-10-31 OOM** (`sudo journalctl -b -4 -k`, 07:33:09 onward). Global OOM:
  `inactive_anon` 14.86 GB, `Free swap = 0kB`. The task dump shows eight `rustc`
  (rss 1.4-1.9 GB, `oom_score_adj 0`) and two `git` from a build outside the graphical
  session. The victims, in order: `claude` (adj 200), `wireplumber` 9 MB, the greeter user's
  `xdg-desktop-portal(-gtk)` 6 MB, then `pipewire`, `pipewire-pulse`, `wireplumber`,
  `gcr-ssh-agent` and `dbus.service` again and again until 07:37:46. That makes 59 kills, 6
  of them the session bus. Badness is `rss + adj × totalpages / 1000`. At 200 that is ≈196k
  pages for a 10 MB service, against ≈119k for a 1.9 GB `rustc` at 0. systemd restarts each
  victim, and the kernel kills it again. Builds now run under `~/bande-linux/bin/gentle` (6 GB
  cap), which removes this trigger for agents, but not for a user.
- **Why the session dies with the bus**: the user bus is `dbus.service` from
  `dbus-user-session`. After a restart, no Bande service, portal or app reconnects. On
  2026-10-31 the shell logged "apps closed for lack of memory go untold" for the rest of the
  day.
- **Decision 0004's layers are inert.** `/proc/config.gz`: `# CONFIG_PSI is not set`,
  `# CONFIG_ZRAM is not set`. The shell logs `no warning when memory runs low: No such file
  or directory` (no `/proc/pressure/memory`). systemd-oomd is not installed and needs PSI.
  zram-generator finds no module. The image installs neither (`--no-install-recommends`). See
  [swap](swap.md).
- **Unclean boot ends**: boots -11 (2026-10-30 20:43) and -6 (2026-10-31 00:23) end with no
  shutdown in the journal and no kernel message. The next boot followed within 90 s, which
  looks like a forced reset. The cause is unknown, and nothing was recorded. The kernel has
  `# CONFIG_SOFTLOCKUP_DETECTOR is not set` and `# CONFIG_DETECT_HUNG_TASK is not set`.
  `RuntimeWatchdogUSec=0` although `/dev/watchdog0` (apple_wdt) exists, and
  `kernel.panic = 0`, so a panic or a hard hang sits forever.
- **The compositor is the session.** `bande-compositor.service` `BindsTo=graphical-session.target`,
  and the release profile has `panic = "abort"`. When it exits, `bande-session` exits and
  greetd shows the login window. Apps die with their Wayland connection. That is the macOS
  model (a WindowServer crash logs out). The shell and the other session clients restart
  inside the compositor (`session.rs` `keep_session_clients_running`, throttled), and the
  session services have `Restart=on-failure`. **A hung compositor is never noticed**:
  `Type=notify` sends `READY=1` (`environment.rs`), but there is no `WatchdogSec`. A
  deadlocked main loop freezes display and input until a power-button reset.
- **App crashes and hangs are handled**: "X quit unexpectedly" with reopen
  (`bande-xdg`, `app-crashed`), ping-based "not responding" with Force Quit
  (`watchdog.rs`), and the OOM-kill notification (decision 0004 §4).
- **Apps to reopen** are saved on logout and shutdown (`app.rs` `remember_apps_to_reopen`), not
  continuously. After a compositor crash, nothing is reopened at the next login.

## Recommendation

1. **Kernel**: `PSI=y`, `ZRAM=m` (+ zsmalloc, lz4/zstd backends, `CRYPTO_ZSTD=y`), `ZSWAP=y`
   (off), plus `SOFTLOCKUP_DETECTOR=y` and `DETECT_HUNG_TASK=y` (report only, no panic), so
   the next freeze leaves a trace.
2. **Image**: install `systemd-oomd` and `systemd-zram-generator`. Enable the hardware
   watchdog (`RuntimeWatchdogSec=30s`), so a hard kernel hang reboots instead of freezing, and
   set `kernel.panic=10` to reboot ten seconds after a panic.
3. **Desktop, kernel-OOM ordering**: set the few session-critical user services to
   `OOMScoreAdjust=100`, the floor a user manager allows (the same as the compositor):
   `dbus.service`, `pipewire.service`, `pipewire-pulse.service`, `wireplumber.service`,
   `xdg-desktop-portal.service`. Their badness drops from ≈196k to ≈98k pages, so anything over
   about 1.5 GB at adj 0, and every big app at 200, dies before the session bus. Ship these as
   vendor drop-ins in `bande-desktop`, the way decision 0004 ships `app.slice.d`.
4. **Desktop, compositor watchdog**: `WatchdogSec=20s` on `bande-compositor.service`. The
   compositor sends `WATCHDOG=1` from a timer in its own event loop at half the interval. A
   stalled loop then gets SIGABRT, the session ends, and the login window comes back. macOS's
   WindowServer watchdog does the same. 20 s is far above any legitimate stall (a DCP modeset
   takes about 1 s).
5. **Keep the logout model** for a compositor crash. Restarting the compositor in place loses
   the apps anyway: Wayland clients cannot reconnect. It would also need the lock state carried
   across, or a crash while locked becomes a lock bypass. Instead, save the apps to reopen
   continuously, so the next login brings them back.

## Milestones

1. **Kernel config** (1 h, `linux`). Recommendation 1 in `bande-headless.config`. Check that
   every line survives `defconfig` + fragment, and build `Image modules` with `kbuild`.
2. **Image packages and watchdog** (1 h, `images`). `systemd-oomd`, `systemd-zram-generator`
   (with the other missing packages, plans/README "Packages on the machine"),
   `etc/systemd/system.conf.d/bande-watchdog.conf` and `etc/sysctl.d/bande-panic.conf`.
3. **Session services at the OOM floor** (30 min, desktop). The drop-ins from recommendation 3,
   a sentence in decision 0004 §2, and the kernel requirements in `docs/debian-readiness.md`.
4. **Compositor watchdog** (1-2 h, desktop). `WATCHDOG_USEC`/`WATCHDOG_PID` parsing, a calloop
   timer that sends `WATCHDOG=1`, and `WatchdogSec=20s` in the unit, with tests for the parsing
   and the message.
5. **Owner test at the mini** (30 min, person, after installing the kernel). Enable oomd and
   zram. `zramctl` shows zram0 (about 8 GB, zstd), and `oomctl` lists `app.slice` at 40 %. In a
   Bande terminal, run `tail /dev/zero` (it buffers one endless line). The low-memory
   window appears, then oomd closes the terminal, the notification names it, and the session
   and audio survive.
6. **Reopen apps after a crash** (1-2 h, desktop). Write the apps-to-reopen file when an app
   opens or closes (debounced), not only at logout, and reopen after an unclean session end.
7. **Crash evidence** (1 h, images, decision). `systemd-coredump` keeps cores of the compositor
   and shell, and the journal stays persistent (it already is). Look into ramoops on the M2
   (needs a reserved-memory region in the DT that survives a watchdog reset).

## Status

- Milestones 1-4: see the gap's status line in `NOTES/gaps2.md`.
- Milestone 5 needs the new kernel installed and a reboot.

## Decisions for the owner

1. **Compositor crash: log out (recommended) or restart in place?** Restarting loses the
   apps anyway and needs lock-state handover.
   Decided overnight 2026-11-01: log out (plan recommendation; owner may revisit).
2. **Watchdog interval**: 20 s (recommended) or longer. Too short kills a compositor stuck
   behind a slow GPU or DCP call. Too long leaves the user staring at a frozen screen.
   Decided overnight 2026-11-01: 20 s (plan recommendation; owner may revisit).
3. **Hardware watchdog and reboot on panic** in the image: recommended. The risk is a reboot
   when the apple_wdt driver misbehaves, which has not been seen.
   Decided overnight 2026-11-01: on in the image, not applied to the running mini
   (plan recommendation; owner may revisit).
4. **systemd-coredump in the image**: recommended (about 1 MB, and cores are kept under
   `/var/lib/systemd/coredump` with size limits). Still open.
5. **End the session when the session bus dies**, instead of leaving a desktop without D-Bus?
   Recommend not yet: with oomd and the OOM floor it should not happen, and ending the session
   loses all apps. Still open.
