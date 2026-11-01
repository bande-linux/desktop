# Footprint: memory, wake-ups and start-up

## Summary

The session is already close to lean at idle: the compositor wakes only for its systemd watchdog
(0.1/s), the services sleep in `epoll`, dist builds are fat-LTO, `codegen-units = 1`,
`panic = "abort"` and stripped, and fonts are `mmap`ed by fontique. Measurements are in
[`docs/quality/footprint.md`](../../docs/quality/footprint.md). The one idle waker found in the
live session was the battery/Wi-Fi follower re-reading all of NetworkManager on every access
point signal-strength signal; that is fixed (milestone 0). What is left is structural: work
that every bar, part or process repeats, and memory held for pictures that are already on the
GPU or not on screen.

## Findings

- **Wi-Fi strength signals** (fixed): NetworkManager announces an access point's `Strength`
  every 10–20 s. `bande_status::devices` re-read the whole Wi-Fi state for each one (every
  device, every access point, every saved connection's settings, connectivity): 180–275
  context switches in the greeter per signal, 12.8 wake-ups/s averaged over 3.8 h of the live
  greeter, with one access point in range. The bar and the lock screen run the same code.
- **Per-display and per-part duplicates in the shell** (`docs/quality/userland.md` tasks 1 and
  9): each bar starts its own device follower, PipeWire client, tray host, minute timer and
  config follower; Control Center opens another PipeWire client; desktop entries are parsed by
  about ten call sites (bar dialogs, low-memory window, taskbar and menu bar per display,
  notifications, launcher, Control Center, desktop, screenshot, reopen) and the shared-mime-info
  database is loaded 2 + N times. `wip/userland-efficiency` holds a status hub (8a096894) and
  a catalog-sharing commit (f5d6f847) written before the rename; 341 commits of `main` have
  since moved the bar (clock mark, time-zone follower, installed apps, taskbar), so they no
  longer apply.
- **One thread parked per launched app** (task 8, `wip` 32a93ba8): `bande_xdg::launch` opens a
  session connection per launch and blocks a thread in `child.wait()` for the app's life.
- **Property followers re-read everything** (task 7, `wip` a6769ca2): 16 uncached proxies; a
  config change costs Settings 41 `Get` calls.
- **Full-size pictures kept on the CPU after upload**: `bande_graphics::Image` holds an
  `Arc<Pixmap>` for as long as the scene uses it, also on the GPU path, so a full-screen
  wallpaper costs its RGBA size twice on unified memory (CPU copy plus texture): 14.7 MB per
  2560×1440 display. The greeter and lock screen also keep the sharp wallpaper for their whole
  life only to draw the unveil after sign-in: about half of the live greeter's 28 MB of
  anonymous memory.
- **Text pages per binary**: 13 GUI binaries each link their own copy of the toolkit, vello,
  wgpu, parley and zbus (13–21 MB stripped). File-backed pages are shared only within one
  binary, so each running GUI process faults in its own copy of the same code.
- **One executor thread per zbus connection**: the greeter has three `zbus::Connection`
  threads (two were system bus; now one), the portal three. Each is a parked thread, not a
  waker.

- **The shell's file index grows with the home**: `bande_search::FileIndex` keeps a
  `BTreeMap<PathBuf, Entry>` (full path, folded name) for up to 400 000 entries and one inotify
  watch per folder. For this machine's home (180 000 entries in 12 000 folders, mostly
  `~/bande-linux/repos`) the shell holds about 20 MB more than with an empty home (171 → 191
  MiB under `bande-idle`).
- **Blocking D-Bus on UI threads, no timeouts** (from `release-review.md`): Control Center
  makes about ten synchronous NetworkManager/BlueZ/logind/notify calls per panel rebuild on
  the shell's UI thread; the tray and dbusmenu calls and the low-memory window's force-quit do
  the same; no connection sets `method_timeout`, so one hung service freezes the bar,
  launcher, notifications and desktop together.
- **Other UI-thread and per-change work** (same review): Notify sends the whole presentation,
  pixels included, on every change; Software parses its catalogue before its window opens; the
  tray refetches every item one by one on each signal; `mime_for` reads files on the Files UI
  thread; `Trash::is_empty()` reads `/proc/self/mounts` again after every operation;
  `Layouts::load` parses `evdev.xml` on UI threads; the taskbar scans the trash on the UI
  thread; `idle_tick` rebuilds stacks every loop while an idle inhibitor exists; the cursor
  cache reloads when the pointer crosses displays of different scale; the largest staging
  buffer stays allocated for the session; `cascade`/`window_index` are linear per commit.

## Recommendation and milestones

0. **Done 2026-11-01** (`perf/footprint`): a `PropertiesChanged` of an access point that
   carries only `Strength`/`LastSeen` updates the shown network in place (`set_strength`) or
   is ignored, in the bar, lock screen, greeter and Settings; everything else still re-reads
   (10.6 → 0.73 context switches/s for the follower). The greeter follows devices on its one
   system bus connection. Icon themes and folder listings are cached in `bande-xdg` (ported
   from `wip` f6ce554c; 34 742 → 5 765 system calls for 51 lookups). `bande-idle` minimises
   with Super+H.
1. **Status hub** (1–2 h): port 8a096894 onto `main`: `bande_status::hub::StatusHub` starts
   the followers once per process and fans `Update`s out to every bar and Control Center. Win
   with one display: one PipeWire client and its thread (Control Center's); with N displays,
   (N−1) × (PipeWire client, device follower, tray host, timers). Risk medium: bar start-up
   order; `shell/bar/tests/live.rs` covers the fan-out. Do it after the taskbar-width/options
   work lands, which touches the same files.
2. **One catalog in the shell** (2 h): the shell context owns an `Rc` of the desktop entries,
   `Apps` and `MimeDatabase`, reloaded by the existing app-entry watcher; parts stop calling
   `applications()`. Measure shell start-up and RSS with `bande-footprint` before/after.
3. **Launch reaper** (1–2 h): port 32a93ba8: one pidfd reaper thread per process and the
   caller's connection for scope placement.
4. **Property follower helper** (2 h): port a6769ca2 (one `GetAll`, then the signal's
   dictionary; read again only when `invalidated` is non-empty).
5. **Free picture pixels after upload on the GPU path** (2 h, toolkit): keep `Image`'s pixels
   until the renderer has a texture, then hold only the texture; re-decode from the source on
   device loss or a CPU fallback. Saves the RGBA size of every wallpaper and backdrop on the
   GPU path (14.7 MB per 2560×1440 display in the shell's desktop surface). Needs a measured
   check that the CPU path (lavapipe, VMs) keeps its pixels.
6. **Greeter and lock: load the sharp wallpaper when sign-in starts** (1 h): it is drawn only
   while unveiling; start its decode when the password is submitted (PAM takes longer than the
   decode) and fall back to the blurred backdrop if it is late. Saves 14.7 MB while the greeter
   or lock screen waits.
7. **Method timeouts and Control Center off the UI thread** (2 h): give every blocking
   connection a `method_timeout` (a few seconds) through one `bande_bus` constructor, and move
   Control Center's panel reads to a worker with an `Inbox`, the way `follow_devices` works.
   Then the tray and dbusmenu calls.
8. **Per-change work from the review** (1 h each, independent): the notification presentation
   without pixels when only the list changed; cache `Trash::is_empty`'s mount list and refresh
   it on mount changes; `Layouts::load` once per process; the taskbar's trash scan on a
   worker; `idle_tick` rebuilding only on stack changes.
9. **Settings' Bluetooth follower** (1 h): it re-reads every adapter and device on any BlueZ
   signal (5.3 wake-ups/s left in Settings with windows minimised, mostly this); apply
   `PropertiesChanged` of `org.bluez.Device1` in place for RSSI-like properties, as Wi-Fi now
   does, and follow BlueZ only while a Bluetooth pane shows (`wip` d497c49b does this for
   NetworkManager).
10. **A compact file index** (2 h): store each entry as (parent index, name) in arenas
   instead of a full `PathBuf` per entry, and the folded names in one string arena; `within`
   queries walk parent indices. Expect the per-entry cost to fall from about 110 bytes to about
   40; measure with `bande-idle` and the real home before and after.
11. **Shared code between GUI binaries** (decision, not scheduled): a multicall `bande` binary
   or a shared toolkit `dylib` would let the shell, apps and lock screen share text pages.
   Rust has no stable ABI for a `dylib` across separately built crates, and a multicall binary
   changes packaging (one package or symlinks), so measure first: `Pss_File` against `Rss_File`
   of two running GUI processes shows what sharing would save.

## Decisions

- Decided overnight 2026-11-01: milestones 1–4 follow `docs/quality/userland.md`; 5 and 6
  wait on before/after numbers on the mini's GPU, which this run could not take (the owner's
  session was at the greeter and nothing visible may be restarted).
- Open: milestone 11 needs the owner (packaging shape).
