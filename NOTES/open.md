# Open items

What is still open across the desktop, one line each. Delete a line when it is done; open gaps
are in [gaps.md](gaps.md) and [gaps2.md](gaps2.md), open review findings in
[plans/release-review.md](plans/release-review.md). Deleted plans and reviews are in git
(`git show 37b326aa:NOTES/plans/<name>.md`).

## First release (owner)

- Before tagging: delete stale ISO artifacts (the org hit its artifact storage quota on
  2026-10-31) and check apt's `state/owners` lists only apt and kernel.
- Apps first, from apps `main`: `<app>-v0.1.0` for terminal, software, installer, monitor, mail,
  calendar, each published by apt before the next; then desktop `v0.1.0`
  ([ci.md](../docs/architecture/ci.md#releasing)); then switch the apps' desktop pin to `v0.1.0`
  and build an image from the archive.
- No reviewer approves a publish today; add one to apt's `apt-publish` environment if wanted.
- `origin/wip/userland-efficiency` still has the old `ci.yml` that runs on every push: delete
  the file or the branch before pushing it again.

## Owner checks at the mini

- After `rebuild-desktop` and a new login: Ctrl shortcuts everywhere and Super only for the
  system; taskbar icons of third-party apps; resize zones; snapping against the taskbar edge;
  sidebar selection.
- Brightness on the mini: the Control Center and Settings bar and the brightness keys dim the
  HDMI monitor, it stays dimmed after replugging, and idle dimming lowers it and restores it.
- DDC/CI needs a machine with a DDC adapter (`ls /sys/class/drm/*/ddc`): real monitor follows the
  bar and its own buttons show after reopening the panel.
- Force Quit from the system menu with a stopped app (`kill -STOP`) marks it within 3 s and kills
  it with its helpers; it opens under real memory pressure; Orca reads it; Shift in the system
  menu shows Force Quit "*App*".
- Notifications after the center moved to its own bus name: banners and center fill and clear,
  Do Not Disturb toggles, Settings ▸ Notifications lists apps, and `GetPresentation` works only
  on `org.bande.NotificationCenter`.
- Power: the resting lock screen (`Super+L`, then clock only; the first input shows the field; a
  minute later it rests again), displays off at the lock stage and after 60 s locked, wake on
  input, the power button, an external display through standby and back (`drm.rs` relink may hit
  EBUSY).
- Reopen after a real crash with `reopen-apps-after-crash #true`.
- Stuck desktop: `Ctrl+Alt+Esc` opens Force Quit; `Shift+Ctrl+Alt+Esc` twice ends the session and the
  login window comes back (`journalctl -u systemd-logind` says "Secure Attention Key", and
  `-u bande-recovery` says it restarted the desktop); after a new login
  `sysctl kernel.panic` says 10 and `systemctl show -p RuntimeWatchdogUSec` says 2min.
- Panic logs on the mini: whether RAM survives its reboot is unknown. Test with one normal reboot:
  `reserve_mem=2M:16384:oops ramoops.mem_name=oops ramoops.console_size=262144` in
  `/etc/kernel/cmdline` and `ramoops` in `modules-load.d`; `/sys/fs/pstore` then has the previous
  boot's console if it does. Ship it in the images only then.
- Recovery on the mini: `Ctrl+Alt+Fn+F2` reaches a text console and `Ctrl+Alt+Fn+F7` comes back
  (also from the login window); `kill -STOP` of `bande-shell` shows "Taskbar & Desktop (Not
  Responding)" in Force Quit and Relaunch brings the bar back; `kill -STOP` of `bande-lock` while
  locked brings a new lock screen within about 15 s; after `apt install systemd-oomd` and a new
  login, `oomctl` lists `app.slice` and `systemctl show user.slice -p MemoryLow` says 640M. GPU
  loss recovery (a new Vulkan device after `VK_ERROR_DEVICE_LOST`) is untested on asahi: the
  journal says "starting the renderer again" when it happens.
- Footprint on the real GPU: shell and greeter memory, the greeter's unveil after sign-in, the
  shell's PipeWire clients with two displays.
- Login: no text and no black from power-on to the desktop on the mini (kernel, cmdline, bootloader
  and U-Boot of `gap-login-look`), Space at power-on opens the menu and "(boot messages)" boots with
  text, the bar flicker at login, time to desktop.
- Apple hardware: DDC/CI through DCP's I2C in `appledrm` (external displays are dimmed by the
  compositor until then; needs reverse engineering of the firmware's calls).
- Apple hardware: Night Light gamma on the DCP; overlay planes and direct scanout; the
  `VK_KHR_external_semaphore_fd` release path and dma-buf modifiers on Honeykrisp; the GPU
  fence wait each frame; input-to-photon latency with a camera; swapping or moving the only
  display (review: the last display is never removed).
- Software (apps repo): install and remove a Debian app; catalogue icons once
  `apt-config-icons` is installed.
- The `bande-shell` installed before `a458c1b9` polls `/proc/pressure/memory` itself and spins one
  core at 100 % from login (seen 2026-11-02 03:00); after the rebuild `top` shows it idle.

## Owner decisions

- Light scheme: move it to the dark scheme's slate family, or keep it warm.
- Mouse scroll speed: add a setting (scales wheel deltas in `input.rs`), or not.
- Settings options: drop Scroll method, Trackpad acceleration and Disable while typing; fold
  Variable refresh rate into Refresh rate (the switch writes a key nothing reads); where `reopen-apps` lives (Log Out
  checkbox, always on or always off).
- A click on the clock or Control Center while the launcher is open: open the panel too, or only
  close the launcher (today).
- Several displays: banners on the main display or on the one with the active window.
- Sleep: ten clean cycles with `sudo ~/bande-linux/bin/s2idle-test.sh keyboard` (at least two of
  them `desktop`; the script counts), then set `AllowSuspend=yes` in `crates/services/power/data/sleep.conf`,
  unmask the targets on the mini and remove `suspend-after "never"` from its `config.kdl`. Or wait for
  Asahi.
- The lock screen ignores logind's `Unlock` (`loginctl unlock-session`): honour it or not.
- Naming: the clash with Debian's `bande` DNS resolver packages; the `org.bande.*` domain.
- Product: a welcome assistant; an audio player (Totem or Rhythmbox); which UI languages first.
- Files: tags; Connect to Server.
- Title-bar toolbars stay at 100 % at large text sizes.
- The unused private `set_corner_radius` request: keep or remove.
- Debian policy: make the greeter opt-in instead of taking over greetd.

## Remaining work

- Files: ranked in [plans/files-depth.md](plans/files-depth.md).
- Unmerged branches: `wip/l10n-ready` (strings into Fluent, an en-XA pseudo-locale),
  `wip/terminal-scopes` (a systemd scope per Terminal tab; apps repo now),
  `wip/userland-efficiency` (conflicts with `main`).
- Windowless apps: Hide does nothing; the taskbar icon menu has no New Window; Calendar's and
  Activity Monitor's menus are empty; parked views keep their widget trees.
- Menu button for GTK 3 windows with our title bar: offer `gtk_shell1` with the global menu bar
  capability and import `org.gtk.Menus`/`org.gtk.Actions` (GTK 4 has no global menu); Menu
  Search does not yet follow a third-party window's `org_kde_kwin_appmenu` address, only its
  menu button does.
- The compositor's window menu keeps its own `Labels` to follow text size; give the shared
  overlay label renderer a per-use text size.
- Footprint: the typed property follower; the bar's own desktop-entry parsing; worker threads
  and timeouts for the remaining blocking D-Bus calls (Control Center setters, tray).
- Text size: menus at 200 % and long labels; Activity Monitor's CPU-core strip (apps repo).
- A Super tap or `Ctrl+Alt+T` pressed before the shell connects is lost.
- The low-memory dialog has no test hook: it shows only on a real PSI stall with memory nearly
  exhausted, so a headless walk cannot open it.
- Mail's Settings window (apps repo) has no title or menu button, unlike the other apps'.
- Notifications on several displays (banners follow the decision above).
- Idle wake-ups: Activity Monitor (apps repo, installed 2026-11-01 build) wakes 5–7/s minimised
  and locked (its 2 s sample keeps running, plus D-Bus traffic).
- Frame clock: predict with a high percentile instead of the maximum of the last 16 frames.
- `bande-render`: `VK_EXT_host_image_copy` to copy shm buffers without staging.
- PipeWire 1.4.2 ignores a Props change on a suspended null sink (report upstream); per-device
  volume memory across a WirePlumber restart is unproven.
- Not built yet: user picker and accounts (no Users pane, no fast user switching), crash
  reports, tiling beyond halves and quarters, custom shortcuts, input methods as
  sources, spell checking, colour filters, on-screen keyboard, notifications on the lock screen,
  inline reply, file content search, spring-loading, printers pane, Wi-Fi hidden and Enterprise
  networks, more portals (GlobalShortcuts, RemoteDesktop) and portal permission review, colour
  management, battery charge limit, Do Not Disturb schedules.
- Debian archive upload: asset provenance for the sounds and `emoji.tsv`, man pages, a
  `debian:sid` toolchain build, about 25 Rust stacks to package and 15 to update in Debian,
  then `debian/`, an ITP and a sponsor (full audit:
  `git show 37b326aa:docs/debian-readiness.md`).
