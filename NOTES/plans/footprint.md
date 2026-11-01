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
1. **Done 2026-11-01** (`perf/status-hub`, merged in f1c160b7): status hub, ported from
   8a096894. `bande_status::hub::StatusHub` starts the device, sound, clock, time-zone, tray,
   unread and errand followers once per shell and fans `Update`s out to every bar; Control
   Center takes the hub's `Audio`. With one display it saves Control Center's PipeWire
   client; each further display saves a device follower, a PipeWire client, a tray host and
   the timers. Under `bande-footprint` (no working PipeWire there) the shell has one thread
   more (the relay, 32 → 33); the PipeWire saving needs a live session to show.
2. **Done 2026-11-01, in part** (`perf/catalog`, merged in 3b73a11c): the shell context owns
   one `Rc<RefCell<Apps>>` (MIME database, associations, handlers) shared by every desktop
   surface and the launcher; before, each display's desktop surface (also the
   wallpaper-only ones) and the launcher loaded their own (1.1 MB each). Shell anonymous
   memory 78.6 → 77.5 MiB with one display under `bande-footprint`. Left: the bar's
   `InstalledApps` (menu bar and taskbar, per display, each with its own folder watch) and
   the on-demand `applications()` calls (dialogs, notifications, low memory, screenshot,
   reopen) still parse the desktop entries themselves (272 KB per copy).
3. **Done 2026-11-01** (`perf/launch-reaper`, merged in 5d7cf04e): launch reaper, ported from
   32a93ba8. One `app-watcher` thread per process polls a pidfd per launched app and keeps
   one session connection for scope placement and failure notices; before, every launch
   parked a thread in `child.wait()` for the app's life and opened a new session connection.
4. **Not done**: the property follower helper. Porting a6769ca2 needs a typed dictionary of
   every `org.bande.Config` property (now about 60) next to the interface, and Settings' and
   the portal's readers rewritten on it. A config change costs Settings 41 `Get` calls,
   but config changes are made by the user, so there is no idle cost; left for later.
5. **Done 2026-11-01** (`perf/pictures`, merged in 3f4e3b55): `Image::redrawable` keeps a way
   to draw the pixels again; the GPU renderer drops the CPU copy once the texture is
   uploaded and any later renderer (a new atlas, device loss, the CPU fallback) redraws it.
   The desktop wallpaper redraws from the wallpaper cache, the lock and greeter backdrops
   from the picture. A window also drops its CPU renderer once the GPU draws, so the
   first-frame CPU copy does not stay. `bande-footprint` on lavapipe with
   `BANDE_RENDERER=gpu` (2928×1818): shell anonymous memory 210.3 → 190.2 MiB (−20.3 MiB,
   the wallpaper's RGBA); `BANDE_RENDERER=cpu` unchanged (78.2 / 78.7 MiB), the CPU path
   keeps its pixels. Still to measure on the mini's GPU (AGX): the shell's and greeter's
   RSS in a live session.
6. **Done 2026-11-01** (`perf/pictures`): the greeter decodes the sharp wallpaper when the
   password is submitted, on its own inbox; the unveil falls back to the blurred backdrop if
   it is late. The lock screen already loaded only the blurred one. Greeter under
   `bande-footprint`: anonymous 83.4 → 61.7 MiB on the GPU path (with milestone 5), 47.1 →
   26.3 MiB on the CPU path. Still to check on the mini: that the sharp picture is ready
   before the unveil after a real PAM sign-in (needs someone signing in).
7. **Done 2026-11-01, in part** (`perf/bus-timeouts`): `bande_bus::session()`/`system()` set
   a 25 s `method_timeout` (the D-Bus default reply timeout, long enough for pairing and
   joins); the shell, lock screen and greeter use them, so a hung service no longer freezes
   them for good. Control Center reads Wi-Fi, the current network, the sign-in page,
   Bluetooth and Do Not Disturb on a `control-status` worker (refreshed when the panel opens
   and after a toggle) instead of about ten calls on the UI thread per rebuild. Left: the
   toggles, network menu and join still call from the UI thread (now bounded); Settings
   keeps unbounded connections because Bluetooth pairing waits for the user; the tray's
   errands already run on the hub's worker.
8. **Done 2026-11-01, in part** (`perf/per-change`, merged in 9f8aa7cf): `Trash` keeps the
   mount list and reads `/proc/self/mounts` again only when it signals a change; the
   taskbar checks whether the Trash is empty on a `trash-check` worker and keeps the
   answer for its menu; the bar parses `evdev.xml` once per shell, on first need;
   `idle_tick` skips the stack walk when no pending idle watch obeys inhibitors. Left: the
   notification presentation without pixels (changes the `org.bande.Notifications` wire
   format for the shell and the status follower), the cursor cache on scale changes, the
   staging buffer and `cascade`/`window_index`.
9. **Done 2026-11-01** (`perf/settings-follow`, merged in a22f171b):
   `bande_bluetooth::shown_change` drops `PropertiesChanged` signals that change nothing
   shown (RSSI, ManufacturerData, ServiceData, UUIDs, TxPower, AddressType, Discovering) in
   Settings and in `bande_status::devices` (bar, lock, greeter); Settings follows every
   NetworkManager signal only while the Wi-Fi or Network pane shows (ported from d497c49b).
   During a 60 s discovery near this mini BlueZ sent 115 signals, of which 39 now cause a
   re-read; with discovery off it sent none in 120 s.
10. **Done 2026-11-01** (`perf/file-index`, merged in fc76ec2e): the index is a tree of nodes
   (parent, first child, next sibling, name and folded name as ranges into two string
   arenas, the folded one only when it differs), compacted when more than half is removed.
   On this home (`/home/mikael`, mostly `~/bande-linux/repos`), a release build of the
   index alone: 40.0 → 11.9 MiB, queries 5–11 ms instead of 6–15 ms, same hits.
11. **Shared code between GUI binaries** (decision, not scheduled): a multicall `bande` binary
   or a shared toolkit `dylib` would let the shell, apps and lock screen share text pages.
   Rust has no stable ABI for a `dylib` across separately built crates, and a multicall binary
   changes packaging (one package or symlinks), so measure first: `Pss_File` against `Rss_File`
   of two running GUI processes shows what sharing would save.

## Decisions

- Decided overnight 2026-11-01: milestones 1–4 follow `docs/quality/userland.md`.
- 2026-11-01: 5 and 6 landed with headless numbers (lavapipe as a GPU and the CPU path);
  the live check on the mini's GPU and a real sign-in is still open.
- Open: milestone 11 needs the owner (packaging shape).
