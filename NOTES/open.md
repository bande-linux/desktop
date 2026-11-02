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
- The image is Debian 13 (trixie) but the archive is built for Debian testing: move the image to
  testing, or also build a trixie set.
- No reviewer approves a publish today; add one to apt's `apt-publish` environment if wanted.
- `origin/wip/userland-efficiency` still has the old `ci.yml` that runs on every push: delete
  the file or the branch before pushing it again.

## Owner checks at the mini

- After `rebuild-desktop` and a new login: Ctrl shortcuts everywhere and Super only for the
  system; taskbar icons of third-party apps; resize zones; snapping against the taskbar edge;
  sidebar selection.
- Force Quit: `Ctrl+Alt+Escape` with a stopped app (`kill -STOP`) marks it within 3 s and kills
  it with its helpers; it opens under real memory pressure; Orca reads it; Shift in the system
  menu shows Force Quit "*App*".
- Notifications after the center moved to its own bus name: banners and center fill and clear,
  Do Not Disturb toggles, Settings ▸ Notifications lists apps, and `GetPresentation` works only
  on `org.bande.NotificationCenter`.
- Power: displays off at the lock stage and after 60 s locked, wake on input, the power button,
  an external display through standby and back (`drm.rs` relink may hit EBUSY).
- Reopen after a real crash with `reopen-apps-after-crash #true`.
- Recovery on the mini: `Ctrl+Alt+Fn+F2` reaches a text console and `Ctrl+Alt+Fn+F7` comes back
  (also from the login window); `kill -STOP` of `bande-shell` shows "Taskbar & Desktop (Not
  Responding)" in Force Quit and Relaunch brings the bar back; `kill -STOP` of `bande-lock` while
  locked brings a new lock screen within about 15 s; after `apt install systemd-oomd` and a new
  login, `oomctl` lists `app.slice` and `systemctl show user.slice -p MemoryLow` says 640M. GPU
  loss recovery (a new Vulkan device after `VK_ERROR_DEVICE_LOST`) is untested on asahi: the
  journal says "the GPU was lost; starting the renderer again" when it happens.
- Footprint on the real GPU: shell and greeter memory, the greeter's unveil after sign-in, the
  shell's PipeWire clients with two displays.
- Login: the greetd console frame before the greeter (`vt.global_cursor_default=0` or a greetd
  change), the bar flicker at login, time to desktop.
- Apple hardware: Night Light gamma on the DCP; overlay planes and direct scanout; the
  `VK_KHR_external_semaphore_fd` release path and dma-buf modifiers on Honeykrisp; the GPU
  fence wait each frame; input-to-photon latency with a camera; swapping or moving the only
  display (review: the last display is never removed).
- Software (apps repo): install and remove a Debian app; catalogue icons once
  `apt-config-icons` is installed.

## Owner decisions

- Light scheme: move it to the dark scheme's slate family, or keep it warm.
- Mouse scroll speed: add a setting (scales wheel deltas in `input.rs`), or not.
- Settings options: drop Scroll method, Trackpad acceleration and Disable while typing; fold
  Variable refresh rate into Refresh rate (the switch writes a key nothing reads); dim shortly
  before the display turns off instead of Dim after; where `reopen-apps` lives (Log Out
  checkbox, always on or always off).
- A click on the clock or Control Center while the launcher is open: open the panel too, or only
  close the launcher (today).
- Several displays: banners on the main display or on the one with the active window.
- Sleep: turn it back on after ten clean s2idle cycles, or wait for Asahi.
- The lock screen ignores logind's `Unlock` (`loginctl unlock-session`): honour it or not.
- Naming: the clash with Debian's `bande` DNS resolver packages; the `org.bande.*` domain.
- Product: a welcome assistant; an audio player (Totem or Rhythmbox); which UI languages first.
- Files: tags; Connect to Server.
- Title-bar toolbars stay at 100 % at large text sizes.
- The unused private `set_corner_radius` request: keep or remove.
- Debian policy: make the greeter opt-in instead of taking over greetd.
- Shared code between the GUI binaries (a multicall binary): only if it saves 20 MB at idle.

## Remaining work

- Files: ranked in [plans/files-depth.md](plans/files-depth.md).
- Unmerged branches: `wip/l10n-ready` (strings into Fluent, an en-XA pseudo-locale),
  `wip/terminal-scopes` (a systemd scope per Terminal tab; apps repo now),
  `wip/userland-efficiency` (conflicts with `main`).
- Windowless apps: Hide does nothing; the taskbar icon menu has no New Window; Calendar's and
  Activity Monitor's menus are empty; parked views keep their widget trees.
- Menu button for GTK 3 windows with our title bar: offer `gtk_shell1` with the global menu bar
  capability and import `org.gtk.Menus`/`org.gtk.Actions` (GTK 4 has no global menu); the
  separate menu bar and Menu Search do not yet follow a third-party window's
  `org_kde_kwin_appmenu` address, only its menu button does.
- The compositor's window menu keeps its own `Labels` to follow text size; give the shared
  overlay label renderer a per-use text size.
- Footprint: the typed property follower; the bar's own desktop-entry parsing; worker threads
  and timeouts for the remaining blocking D-Bus calls (Control Center setters, tray).
- Text size: menus at 200 % and long labels; Activity Monitor's CPU-core strip (apps repo).
- A `Super+Space` pressed before the shell connects is lost.
- Stuck state still open: windows maximized while the bar was gone stay under it when it
  returns (`fit_to_usable_areas` refits on every layer commit and destroy, so the cause is
  elsewhere; reproduce first).
- Notifications on several displays (banners follow the decision above).
- Idle wake-ups left (`bande-idle`, 2026-11-02): Activity Monitor 6–8/s also while minimised and
  locked (apps repo; its 2 s sampling should stop off screen); Settings 1.3/s with its window open.
- Blocking work on shell UI threads: the desktop's `relist` (`read_dir` of Desktop on every
  watched change), the screenshot engine's `recv()` without timeout, Control Center artwork
  decoding and app-icon lookup on player updates, the launcher's `recently-used.xbel` read on
  each open.
- Look (for the owner): the en-US taskbar day line reads "PM · 25 Wed" (ICU's short day order;
  macOS shows "Wed 25"); decide the day format for the two-line clock.
- Frame clock: predict with a high percentile instead of the maximum of the last 16 frames.
- `bande-render`: `VK_EXT_host_image_copy` to copy shm buffers without staging.
- PipeWire 1.4.2 ignores a Props change on a suspended null sink (report upstream); per-device
  volume memory across a WirePlumber restart is unproven.
- Not built yet: user picker and accounts (no Users pane, no fast user switching), crash
  reports, hot corners, tiling beyond halves and quarters, custom shortcuts, input methods as
  sources, spell checking, colour filters, on-screen keyboard, notifications on the lock screen,
  inline reply, file content search, spring-loading, printers pane, Wi-Fi hidden and Enterprise
  networks, more portals (GlobalShortcuts, RemoteDesktop) and portal permission review, colour
  management, battery charge limit, Do Not Disturb schedules.
- Debian archive upload: asset provenance for the sounds and `emoji.tsv`, man pages, a
  `debian:sid` toolchain build, about 25 Rust stacks to package and 15 to update in Debian,
  then `debian/`, an ITP and a sponsor (full audit:
  `git show 37b326aa:docs/debian-readiness.md`).
