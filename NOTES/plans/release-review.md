# Pre-release review (v0.1.0), 2026-11-01

A read of the whole workspace before the first release: crash risks reachable from clients or files,
security boundaries, error swallowing, missing timeouts, dead features and dead code. Each finding was
traced end to end (caller, input, guards upstream) before it was listed. Fixes landed on
`review/release`; every fix that could be tested has a test that fails without it.

Severity: **critical** = lock-screen bypass or session-wide compromise; **high** = a client, file
or volume can hang or crash a session component, run code, or delete data; **medium** = wrong
behaviour, unbounded growth or a weaker boundary than documented; **low** = edge cases, hygiene.

Counts: 4 critical, 11 high, 27 medium, about 53 low. Fixed: 28 (3 critical, 8 high, 12 medium, 5 low).

## Fixed

| Sev | Where | Finding | Commit |
| --- | --- | --- | --- |
| critical | compositor `popup.rs` | Any app could send `xdg_popup.grab` while the session was locked and take the keyboard from the lock surface (the grab path set focus directly, skipping the lock redirect), so the password went to the app. Grabs are refused while locked. | `c468b8a2` |
| critical | compositor `input.rs` (smithay input-method grab) | Any client could bind `zwp_input_method_v2` and `GrabKeyboard`; smithay's grab forwards every key to it whatever has focus, the lock password included, and the lock surface got nothing. While locked, keyboard grabs are dropped before a key is delivered. | `c468b8a2` |
| critical | compositor `keyboard_monitor.rs` | `org.freedesktop.a11y.KeyboardMonitor` accepted any session-bus caller: `WatchKeyboard` logged every key including the lock password, `GrabKeyboard` swallowed all keys. Only the owner of `org.gnome.Orca.KeyboardMonitor` is served now, as in Mutter. | `b42459ce` |
| high | portal `screenshot.rs`, `screencast.rs` | The `impl.portal` backend trusted `permission_store_checked` and forged `restore_data` from any caller, so any unsandboxed client could capture the screen silently through `org.bande.Screenshot`'s portal trust. Both are honoured only from the owner of `org.freedesktop.portal.Desktop`. | `53a3dff6` |
| high | greeter `accounts.rs` | The pictures folder is `3733`, so any local user could put a FIFO at `<uid>`; the greeter's blocking `open` then hung before the login window appeared. Opened with `O_NONBLOCK`. | `9ce07e85` |
| high | toolkit `graphics/scene.rs` (`Image::load`) | Notification `app_icon`/`image-path`, MPRIS art and SNI icon paths are client strings; `/dev/zero` read until OOM, a FIFO hung the shell. Images are read only from regular files under 16 MiB, non-blocking. (Decoding still runs on the UI thread, see below.) | `17f58b38` |
| high | compositor `window.rs` | `set_window_geometry` is unvalidated in smithay; a 2^31 width on a server-decorated window made the title bar rasteriser allocate hundreds of GB and abort the compositor. The geometry is clamped to the surface tree, as xdg-shell specifies. | `22a8c44a` |
| high | catalog `dep11.rs` | After a YAML syntax error serde_yaml_ng returns the same error document forever, and the `continue` arm spun Software at 100% CPU before its window opened. Parsing stops at a repeated error. | `344b12a0` |
| high | xdg `mime.rs` | `for_path` opened files to sniff them on the Files UI thread during paint; a FIFO without an extension froze Files. FIFOs, sockets and devices are named by kind (`inode/fifo`, ...) and sniffing opens non-blocking. | `fd357760` |
| medium | preview `lib.rs` | The same FIFO hung the single preview loader thread, and with it every later preview and folder size. Non-regular files preview as Other; decoding has a 256 MiB allocation limit. | `fd357760` |
| high | xdg `apps.rs` (`launcher`) | Double-clicking any `.desktop` file in Files or on the desktop ran its `Exec` (a launcher in Downloads, an archive, a USB stick). Only entries in an `applications` data dir, or owned by the user with an executable bit, launch now; others open with the default app. | `2671d5d9` |
| high | fs `trash.rs` | Volume trash folders were trusted without link or owner checks: a stick with `.Trash-1000/files -> ~` made Empty Trash delete home folders, and Put Back moved real folders. A volume trash is used only when its root, `files` and `info` are real folders the user owns (and `.Trash` is not a link). | `24b4e9e1` |
| medium | compositor `layer` namespaces (`handlers.rs`) | Any app could name its layer `menubar`/`bar`/`taskbar` and get panel placement, whole-screen popups and the Super+F2/F3 focus. Non-shell layers get an `app/` prefix. | `04274fc5` |
| medium | polkit `main.rs` → new `bridge.rs` | A request cancelled by polkitd while waiting behind another dialog later took the dialog and then left `active` set forever, so every later authentication hung; identical requests compared equal. Requests carry their cookie, a waiting request is withdrawn on close, and close always runs. Bridge moved to the library with a test. | `e82ecc90` |
| medium | compositor `displays.rs` | A window menu open on a display that is unplugged kept a stale screen index; the next Right arrow or submenu hover indexed `screens` out of range and crashed the compositor. The menu closes when displays change. | `b583eab2` |
| medium | toolkit widgets `editor/mod.rs` | A screen reader `SetTextSelection` during IME composition stored a layout offset past the real text; the next edit panicked the app. Refused while composing; run ranges are bounds-checked. | `fe22a994` |
| medium | notify `center.rs` | `expire_timeout=0` notifications accumulated forever (memory and `PresentationChanged` size). At most 200 are kept; the oldest are closed. | `2a577413` |
| medium | notifications `client.rs` | The icon cache keyed by client strings was never pruned. Bounded like the picture cache. | `d8707562` |
| medium | compositor `management.rs` | `wlr-output-management` `SetScale` accepted any positive value (inf, near zero). Limited to the Settings range (`SCALE_RANGE`). | `4a178c8f` |
| medium | taskbar-model `bus.rs` | A NaN `progress` from any `LauncherEntry` sender kept the taskbar spring animating forever. Non-finite progress is ignored. | `55dbef13` |
| medium | terminal / toolkit `Window::set_title` | OSC 0/2 titles are unbounded; a title over the Wayland message size would get the terminal disconnected with all its tabs. Titles are capped at 1 KiB on a char boundary. | `2f5dad8b` |
| low | search `calc.rs` | A long run of `(` or `-` pasted into the launcher recursed until the shell's stack overflowed. Input over 256 tokens is not a calculation. | `cc28a40f` |
| medium | render `lib.rs` | Client shm/dma-buf sizes above `maxImageDimension2D` reached `vkCreateImage` (undefined behaviour). Refused with `ImageTooLarge`. | `2517170b` |
| medium | compositor `main.rs`, `input_settings.rs` | A `config.kdl` read or parse error silently became defaults and was applied mid-session (layouts, sticky keys, display scales reset). The current settings are kept and the error is logged. | `720496b8` |
| low | toolkit `presentation.rs` | `now + (at - clock)` and `Duration::new` panicked on a far-future presentation time. Saturating. | `4a8ee9b2` |
| low | installer `deb.rs`, status `tray.rs` | `Installed-Size * 1024`, tray `width * height * 4` and `(width - 32).abs()` overflowed on crafted values. Saturating/checked. | `4a8ee9b2` |
| low | greeter `Cargo.toml` | Unused `jiff`. Removed (dev `rustix` is used by a shared test module). | `102e3c9e` |
| low | `bande-session`, docs, ci | "an Bande" left from the rename (one shown to users). Fixed. lock `Cargo.toml` said Control-Super-Q; it is Super+L. | `102e3c9e` |

## Not fixed

### Critical / high, need a design decision

- **critical — virtual keyboard and input method are open to every client** (`state/globals.rs:127`, filters `|_| true`). Fixed above for the lock screen's keys, but outside the lock any client (Flatpak included) can bind `zwp_input_method_v2` and read every key, and `zwp_virtual_keyboard_v1` types into the focused surface, including the lock surface (password guessing) and terminals. Fcitx5/IBus need both globals, so a plain shell-only filter would break them. Recommendation: implement `wp_security_context_v1` (smithay has it) and refuse these globals, layer-shell Overlay/Exclusive, output management and output power to sandboxed clients; and drop virtual-keyboard keys while locked (needs a wrapper around smithay's dispatch, which has no hook).
- **high — Control Center blocks the shell's UI thread on D-Bus** (`controlcenter/src/client.rs:170-400`): about ten synchronous NetworkManager/BlueZ/logind/notify round trips per panel rebuild, no timeouts; a hung NetworkManager freezes bar, launcher, notifications and desktop together. Same class: tray/dbusmenu calls in `status/tray.rs`, `low_memory.rs:451` `force_quit`. No connection in the workspace sets `method_timeout`. Move to worker threads with an inbox (as `follow_devices` does) and set method timeouts. Reported to the performance agent as well.
- **high — images are still decoded on the shell UI thread** (notifications, MPRIS art, SNI icons): the size cap above bounds it, but a 16 MiB cover still stalls the shell per track change. Decode on a worker.
- **high — the last display is never removed and a reconnect on the same connector keeps the old config** (`displays.rs:240, 302`): swapping the monitor on one connector keeps the old mode (DRM renders at the old size), and moving the only display to another connector leaves the dead one as the main screen, so the lock screen's password field can land on the invisible output. Needs a placeholder screen and identity-aware replacement; hardware-dependent, test on the mini.
- **medium — any shell-role client can unlock the session** (`lock.rs:185-237`): a second `ext_session_lock_v1` from any `SessionRole::Shell` client (shell, screenshot, dialogs, review) can send `unlock_and_destroy`; smithay calls `unlock()` without saying which lock. Give bande-lock its own role and filter the lock global to it.
- **medium — `org.bande.Clipboard` serves seven days of history to any bus caller** (`clipboard/src/bus.rs:35`). Same-user unsandboxed processes can read the on-disk history anyway; Flatpak apps need an explicit talk permission. Restrict to the shell when the shell owns a bus name.
- **medium — clipboard daemon reads offers one MIME at a time on its only loop** (`clipboard/src/main.rs:248`): no total deadline and no MIME count cap, so a client offering many silent MIME types stalls pastes; `pipe()` EMFILE ends the daemon. Low: an offer whose password-hint MIME timed out is recorded to disk (`main.rs:201`); undecodable and `.partial` history files are never deleted (`history.rs:121`).

### Medium

- compositor: `wlr-output-management` `SetMode` accepts another head's mode and `use_mode` returning `Ok(false)` is silent (`management.rs`, `main.rs:187`). No test client for the protocol yet.
- toolkit clipboard: paste/drop reads have no size cap, no timeout and readings never expire (`platform/src/clipboard.rs:333, 487`).
- notify: `rule_for` remembers every new `app_name` forever and rewrites `notification-apps` each time (`lib.rs:574`); cap apps without a user rule.
- fs: the shared `.Trash/$uid` path is checked now, but `Path=` in a volume `.trashinfo` may be absolute or contain `..`, so Put Back can create files anywhere the user can write (`trash.rs:122`, low).
- fs `listing.rs:184`: paths are built from the lossy UTF-8 name, so non-UTF-8 names cannot be opened, copied or trashed; use `DirEntry::path`.
- thumbnails `generate.rs:182`: decoding without `Limits` (512 MiB default each, three at once, `panic = "abort"` in dist).
- volumes `eject.rs:44`: a busy eject only logs; the user may pull a mounted drive.
- Settings: the Variable refresh rate switch writes a key nothing reads; the compositor has no VRR. Remove the row or implement VRR (touches files the taskbar-options agent is changing).
- bar `menu_bar_client.rs:488`: the tray icon cache is keyed by the Debug text of the pixel buffer and never pruned; animated tray icons grow it.
- desktop `client.rs:306, 898` [taskbar agent's area]: the desktop keeps the bar edge and icon size from when it was created.
- desktop `client.rs:785` / wallpaper `choice.rs:71`: a configured wallpaper that is missing never falls back to the default, and a failed load is never retried (removing it from `pending` naively would reload in a loop).
- keyboard `layouts.rs:68, 112`: an XML or read error gives an empty layout list with no log.
- greeter/config `login_picture.rs:32`: `SetWallpaper*` copies any readable file into the greeter folder (readable by `_greetd`); another local user can squat `<uid>` so the rename always fails (logged at info). Share only files that decode as images; log at warn.

### Low

- compositor: `access_keys.rs:182` leaks a thread per Shift-five-times prompt; `recall.rs:44` takes app_ids with tabs/newlines unescaped and never prunes entries; blocking D-Bus calls without timeouts at startup (`main.rs:405`, `environment.rs:204`); `output_power` open to all and controls not pruned on destroy; `constraint.rs:146` cursor hint unbounded; `bande_app_v1.show_emoji_picker` accepted from any client at any time; `keyboard_shortcuts_inhibit` inhibitors are activated without consent.
- docs `compositor.md` protocol matrix: pointer constraints, relative pointer and shortcut inhibit are implemented, not "not implemented"; `bande_shell_v1`/`bande_app_v1` versions are 1; input method and virtual keyboard need the trust caveat.
- toolkit: configure-wait loops ignore `closed` (`layer.rs:118`, `window/mod.rs:419, 1093`); a keymap that fails to compile is ignored silently (`seat.rs:511`); a read error ends a paste as if complete (`clipboard.rs:499`); clipboard `send` threads block on `write_all` without a deadline (`clipboard.rs:884`); preedit cursor offsets vs normalised text (`keys.rs:310`, cosmetic).
- lock: PAM info/error messages are dropped and an expired account looks like a wrong password (`lock/src/main.rs:47`).
- polkit: `getent passwd` runs synchronously in the async handler (`agent.rs:166`).
- notify: `read_apps` treats any read error as an empty file, then overwrites the user's rules; the write is not atomic (`lib.rs:316`).
- config `service.rs:63`: `replace()` does not fsync, does not keep the old mode and replaces a symlinked `config.kdl` with a file.
- network: `replace_password` overwrites a saved PSK before it is verified (`lib.rs:333`); `wait_for_activation` has no deadline (`lib.rs:437`).
- installer: the previewed `.deb` is re-read by PackageKit at install time (TOCTOU by a same-user process; copy into a private folder).
- tray `lib.rs:122`: register-then-disconnect race leaves an item; sound: unlimited `Play` streams, non-drained streams never removed (`audio/graph.rs:451`); power: no per-client `Inhibit` cap (`screensaver.rs:120`).
- Software `worker.rs:59`: the removal confirmation omits `Obsoleting` packages.
- catalog `xml.rs:66`: description XML errors become an empty description; DEP-11 document errors log at debug only.
- volumes `mounts.rs:31`: lossy mount-point unescape; watch `lib.rs:118`: `IN_Q_OVERFLOW` ignored; status `session.rs:254`: full name read from `/etc/passwd` instead of NSS.
- fs: trashing a name over 245 bytes always fails (`.trashinfo` exceeds NAME_MAX); `archive.rs:375` skips unsafe zip names without a failure; rename/move check-then-rename can overwrite (`ops.rs:756`, use `RENAME_NOREPLACE`).
- terminal `session.rs:96`: replies to a program that never reads stdin grow without bound.
- file-chooser `model.rs:42`: exponential `*` backtracking; reuse `xdg::mime::wildcard_match`. xdg `launch.rs:185`: field codes inside an argument (`--file=%f`) are dropped.
- screenshot `main.rs:303`: the shutter plays before the shot and failures are silent; desktop rename/new-folder errors only log; Settings shows a change as saved when its worker could not connect (`follow.rs:252`); bar app-menu titles and nodes count differently when the first child is invisible (`menu_bar_client.rs:266`); taskbar `launcher` map grows from spoofed LauncherEntry ids; `Context::config()` drops errors (`shell/shell.rs:51`).
- Dead code: `packages::details_local`, spacing `XXL`/`XXXL`, the `Command::Reveal(_) => false` arm in Settings `follow.rs:375`; test-only pub fns in Settings; 13 binary crates listed in `[workspace.dependencies]` that nothing depends on; clipboard's direct `wayland-backend` dependency is probably redundant.

## Performance notes (for the performance audit)

Notify sends the whole presentation, pixels included, on every change; Software parses the catalogue on the main thread before its window; the tray refetches every item serially on each signal; `mime_for` reads files on the Files UI thread; `Trash::is_empty()` rereads `/proc/self/mounts` after every operation; `Layouts::load` parses evdev.xml on UI threads; the taskbar scans the trash on the UI thread; `idle_tick` rebuilds stacks every loop while an idle inhibitor exists; the cursor cache reloads on crossing displays of different scale; the largest staging buffer stays allocated for the session; `cascade`/`window_index` are linear per commit.

## Checked and sound

The `is_shell` trust (socket pairs only, `WAYLAND_SOCKET` not inherited); capture, foreign-toplevel, session-lock and data-control globals; move/resize/menu serials; lock-client crash keeps the session locked and blank; greeter starts only `bande-session`; polkit accepts calls only from polkitd and switches only to listed identities; PAM buffers are zeroised; notify `image-data` parsing is bounds-checked; Exec splitting never uses a shell; `remove_all` does not follow links; zip extraction refuses paths through links; Activity Monitor signals by pidfd with a start-time check; no `unwrap`/`expect`/`todo!` in production code; packaging assets and Exec lines all resolve.

## Component review 2026-11-01 evening: apps

Files, Settings and the file chooser here, and every app in bande-linux/apps (Terminal, Activity
Monitor, Software, Package Installer, Mail, Calendar, the template, `ci/`), after today's move onto
bande-app. Mail and Calendar only for the move.

### Fixed

| Severity | Where | What | Commit |
|---|---|---|---|
| medium | Settings `main.rs` | The bande-app view never reported `animating`, so toggles, sliders, the spinner and progress froze after their first frame until the next input. | `544d965e` |
| medium | apps: Software, Package Installer `main.rs` | Same: no `animating`, so spinners, progress bars and scroll momentum stopped; Software's search field also gave no text input area. | apps `35c268f` |
| medium | apps: Calendar `main.rs` | An `.ics` opened from a later launch is now read by the running Calendar on its UI thread: a FIFO hung it and a huge file was read whole. `ical::read_file` opens non-blocking, takes only regular files up to 64 MiB. | apps `0225635` |
| low | file chooser `main.rs` | No text input area for the name and location fields (input method, on-screen keyboard); its own staging write is now `bande_xdg::replace_file`. | `6abaa123` |
| low | apps: Monitor, Terminal `Cargo.toml` | Unused `bande-log` (bande-app initialises logging). | apps `56bb215` |

### Not fixed

- Owner decision: closing the main window quits Activity Monitor and Mail (as before the move); on macOS Mail stays running.
- low, Activity Monitor: the `Screen` trait has no `text_input_area`, so its search field gives the input method no area; needs a trait method in two views (polish agent is in those files).
- low, Settings `view.rs:1439`: `deadline()` covers only the display countdown, not `Ui::deadline()` (tooltip delays); bande-app's `View for Ui` forwards neither `deadline` nor `tick` either (libs).
- low, Settings `services.rs`: while a network pane is open both NetworkManager rules match the root path, so its signals are read twice.
- low, Files `main.rs` `open_window`: every window loads `Apps` (all .desktop files) and the thumbnailer again on the UI thread; one process could share them.
- low, Terminal: a settings change is loaded once by the app and again by every window (`reload_settings`).
- low, every bande-app app: `changed` refreshes the menus after each handled event, so the menu model is rebuilt per pointer move (diffed before D-Bus, CPU only).
- low, file chooser `model.rs:71`: a portal app id with a tab or newline is written unescaped into `file-chooser-folders`.

### Checked and sound

Terminal: one process, per-window restore files `<pid>-<n>`, badges summed per window, quit asks once for every busy tab; Files: `--new-window`, `--info`, relative paths from later launches, tab vs window close; Monitor sampling stops while every window is suspended; Mail keeps drafts on close and quit; Calendar's deadlines always advance; Installer opens a window per package from later launches; `ci/check-app.sh`, `new-app.sh`, `gate.sh`, `build-app.sh` and the template.

## Component review 2026-11-01 evening: compositor

Scope: `crates/platform/compositor`, `crates/platform/render`, `crates/platform/protocols`, with
today's in-compositor Force Quit, link-status re-modeset and the alert/fixed-size dialog changes
read line by line. No new panics, unused dependencies or leftover "unbound" names.

Fixed:

| Severity | Where | Defect | Commit |
| --- | --- | --- | --- |
| medium | `displays.rs` | Force Quit closed only when its screen index went out of range. Unplugging the display it was on while another display took index 0 left it open at its old place, invisible but taking every key. It now closes on any display change, as the window menu does. | `e39e298d` |
| low | `lock.rs` | Force Quit and the window menu closed on the next loop tick after a lock, so keys handled in the same dispatch as the lock request went to them instead of the lock surface. Both close when the lock starts. | `19c90f72` |
| low | `docs/architecture/compositor.md` | Said shell clients get `app_responding` so Force Quit can mark the app; Force Quit is now the compositor's own. | `36ebb784` |

Not fixed:

- medium (hardware) — `drm.rs` `relink` commits `link-status = GOOD` as its own connector-only
  atomic commit outside smithay's `DrmCompositor`; with a page flip pending it can fail with
  EBUSY (logged; `reset_state` still forces a full modeset on the next frame). Needs the mini
  with an external display through standby, not a code guess.
- low — `force_quit_due` rebuilds the app list (catalog lookups, `/proc/<pid>/comm` for apps
  without a name) on every loop iteration while Force Quit is open, and a passed toolkit
  deadline returns a zero timeout until the next frame paints. Only while the window is open;
  returning no deadline instead would miss the repaint's new one, since the tick runs before
  the frame schedulers.
- low — a second button pressed inside Force Quit replaces `held`, so the first button's release
  goes to the window under the pointer; closing Force Quit by key while a button is held does
  the same. Clients ignore an unmatched release.
- note — any AT-SPI client can press Force Quit's buttons while it is open, as with any app's
  accessibility tree.
- docs outside this component: `docs/architecture/session.md` (lines 351, 447) still lists Force
  Quit under a planned `org.bande.Watchdog`.

## Component review 2026-11-01 evening: shell

Scope: `crates/shell/*`, weighted on today's status hub, Control Center worker, wallpaper redraw, greeter lazy wallpaper, shared app catalogue, Trash check, dialogs and notification placement.

Fixed:

| Severity | Where | Defect | Commit |
|---|---|---|---|
| medium | greeter `main.rs`, lock `backdrop.rs` | The greeter handed the backdrop worker `/proc/self/fd/N` of the login picture and the new redrawable wallpaper kept that path for redraws. Choosing another person closed the file while the old blurred wallpaper stayed on screen (and a worker could still be starting), so N could be reused by the next file or an `Inbox` pipe; a redraw then read the wrong picture or blocked the login window reading a pipe. The worker and every redraw now hold the opened file. | `2dc9a91d` |
| low | lock `backdrop.rs` | The lock's wallpaper lookup still opened an unbounded session bus connection. Uses `bande_bus::session`. | `2dc9a91d` |
| medium | bar `menu_bar_client.rs` | The tray icon cache was keyed by the `Debug` text of each pixel buffer (tens of KB per key, formatted on every refresh) and never pruned, so an animated tray icon grew the shell without bound. Only the current icon of each item is kept. | `2c1c39a2` |

Not fixed:

- medium — Control Center still runs `scan`/`networks`/`sign_in_page` (`choose_network`, `join`) and the Wi-Fi, Bluetooth and Do Not Disturb setters on the shell's UI thread; now bounded by the 25 s bus timeout, not removed. Needs the setters moved onto the `control-status` worker; a design change rather than a launch fix.
- low — Control Center shows the Wi-Fi/Bluetooth state from the last time it was opened for the first frame, then the worker's fresh read (the panel can change height once). No signal following while it is open; external changes show on the next open.
- low — the status hub broadcasts `Update::Menu` replies to every bar; with several displays a bar that still thinks a menu is open could show another bar's tray menu at its own anchor. Needs a requester id on `Errand::ShowMenu` (in `bande-status`, outside this component).
- low — Trash checks run on a new thread per change; two overlapping checks can finish out of order and leave the stale answer. Changes are debounced, so rare.
- low — redrawable wallpapers are redrawn synchronously on the UI thread when the GPU drops them (atlas growth); a full decode and blur in the lock or greeter. Toolkit behaviour, not changed here.
- low — the lock's wallpaper is read with `fs::read` of the user's own configured path (no size or file-kind check); self-inflicted only.
- Earlier open shell items (tray/dbusmenu calls and `force_quit` on the UI thread, image decoding on the UI thread, PAM messages, desktop bar edge, screenshot shutter) are unchanged.

## Component review 2026-11-01 evening: services

Scope: `crates/services/*` (automount, clipboard, config, notify, polkit, portal, power, sound,
tray), with today's display-off/lock-stage power changes, the polkit bridge and the portal's
frontend trust read line by line. No panics, `unwrap`/`expect` or leftover "unbound" names in
production code.

Fixed:

| Severity | Where | Defect | Commit |
| --- | --- | --- | --- |
| high | notify `lib.rs`, `client.rs` | `org.bande.NotificationCenter` was served on the `org.freedesktop.Notifications` connection, and Flatpak's proxy grants a name's whole connection. Any app allowed to post notifications (Signal, Discord, Telegram…) could read every other app's notifications (`GetPresentation`, `PresentationChanged`: codes, messages), invoke their actions, clear them and switch Do Not Disturb or app rules. The center now has its own connection and name (`org.bande.NotificationCenter`, D-Bus activatable), requested before the specification name; the specification's signals still come from the specification's connection. | `3c36a36c` |
| medium | clipboard `main.rs`, `history.rs` | A copy whose `x-kde-passwordManagerHint` (or concealed type) did not answer in time was kept in the on-disk history, so a slow password manager's password was written to disk. Such a copy is now treated as a secret. | `ad638cfc` |
| low | config `service.rs` | `replace()` turned a linked `config.kdl` (dotfiles) into a plain file, reset its mode and did not sync before the rename. It writes through the link, keeps the mode and syncs. | `0ac0249f` |
| low | portal `gtk.rs` | The same for the user's own `gtk-3.0`/`gtk-4.0` `gtk.css`, where the portal adds its import: a linked stylesheet was replaced by a copy. | `4ac83ca1` |
| low | clipboard `Cargo.toml` | Unused direct `wayland-backend` dependency. | `b67dfbeb` |
| medium | automount `lib.rs` | `HintAuto` volumes were mounted while the session was locked or inactive, exposing the kernel to an untrusted filesystem at the lock screen. Owner decision 2026-11-01: like GNOME, automount only while the logind session is `Active` and not `LockedHint`; volumes that appear meanwhile are mounted on unlock unless removed. | this branch |

Not fixed:

- medium (design) — `org.bande.Clipboard` still serves the history to any session-bus caller (as
  listed above); Flatpak apps need an explicit talk permission. Its callers are the launcher and
  Settings, so a shell-only check needs a Settings path too.
- low — any client may `CloseNotification` or pass `replaces_id` for another app's notification
  (the specification does not tie ids to senders); remembering the sender would stop a
  sandboxed app replacing another's notification with its own text and actions.
- low — the `org.freedesktop.impl.portal.*` methods (ScreenCast `Start`, Screenshot, Wallpaper)
  accept any session caller; only the restore data and stored permission are frontend-checked.
  An unsandboxed same-user process could call `Start` on a frontend session path first; it can
  read the screen in other ways anyway.
- low — notify keeps summary/body unbounded: 200 large notifications can push
  `PresentationChanged` past the bus's message limit, which then fails until they are cleared.
- low — power: `bande-power` keeps every `wl_output` it ever saw (no `global_remove`), so each
  replug adds one; ScreenSaver `Inhibit` calls logind synchronously inside the async handler;
  commands of two threads (`idle_for` vs `set_inhibited`) run outside the machine lock and can
  interleave (Dim after Undim).
- low — clipboard paste threads (`serve`) block on `write_all` without a deadline; a reader that
  never reads keeps a thread.
- low — automount's `ActionInvoked` match accepts the signal from any sender (worst case: opens
  Storage settings).
- note — `adapter.rs` and `battery.rs` declare the same `UPower` proxy.
- `ci/release/check-debs.sh` (not this component) could also check
  `org.bande.NotificationCenter.service`.

What to check on the machine: after the rebuild, the notification banners and center still fill
and clear, Do Not Disturb toggles from Control Center, Settings > Notifications lists the apps,
and `busctl --user call org.freedesktop.Notifications /org/freedesktop/Notifications
org.bande.NotificationCenter GetPresentation` fails while the same call on
`org.bande.NotificationCenter` works.

## Component review 2026-11-01 evening: libs

Scope: `crates/toolkit/*` and `crates/libs/*`, today's changes first (bande-app, bande-xdg, bus,
network, bluetooth, search, thumbnails, preview, fs, volumes, status hub, packages, app-settings).
Compile-checked per change; no tests added (test policy of 3e32ab75).

### Fixed

| Sev | Where | Finding | Commit |
| --- | --- | --- | --- |
| medium | toolkit `seat.rs` | With Caps Lock on, Ctrl+C arrived as `C` without Shift, so copy, paste, undo, select all, Ctrl+W/Q/, and every menu shortcut did nothing. Letters of Ctrl/Alt/Super keys are cased by Shift only. | `d8fc15b3` |
| medium | toolkit `Drop for Window` | Every window binds its own seat objects; closing an `open_beside` window (every bande-app window, bars, Control Center) left its keyboard, pointer, data device, primary device and text input alive, so each later key, motion and copy still went to every closed window. Released on drop; landed on main independently in `d01af772`, so this branch keeps main's version. | `d01af772` |
| medium | status `devices.rs` | `NameOwnerChanged` was taken from any sender; any local user could send one to the watcher's unique name and freeze battery, Wi-Fi and Bluetooth status in every bar and the lock screen. Only the bus itself is believed. | `89509986` |
| medium | status `tray.rs` (hub) | With one bar per display, each focus change or menu change queued N identical app-menu fetches on the one errand thread, and every reply rebuilt all N bars (N²). Queued fetches for the same app are done once. | `fe30ac1b` |
| low | packages `transaction.rs` | PackageKit's exit was also taken from any sender (a fake signal failed the running install as "PackageKit stopped"); the relay thread outlived early errors on the shared system connection. Bus-only sender; the caller reads the messages itself. | `64f9cef3` |
| low | network `lib.rs` | A network with several access points (dual band, mesh) followed only the strongest one's later strength signals, so it showed that one fading while another was strong. Each access point's strength is kept and the strongest shown. | `9c94f20b` |
| low | network `join` | When the stale-interface check failed (polkit refusing `Update`), its error replaced NetworkManager's refusal. The refusal is kept. | `9c94f20b` |
| low | xdg `launch.rs` | The launched-app watcher used a session connection without a method timeout; a hung systemd stopped reaping and scoping every later app. Uses `bande_bus::session`. | `c1b1c43b` |
| low | toolkit `seat.rs` | A repeat rate of 0 (or above 1 MHz) while a key was held made the loop spin at 100% CPU. Repeat stops when turned off and waits at least 1 ms. | `3b93d805` |
| low | toolkit `window/mod.rs` | The 1 KiB title cap applied only to `set_title`; a window opened with a long title was disconnected. | `31609b4c` |
| low | app-settings | A failed settings save retried and warned on every event, pointer motion included; the wake pipe copied `bande-inbox` with a blocking writer. Retries only when the settings change; uses `Inbox`. | `e4c2028b` |
| low | xdg `reopen.rs` | Duplicate of `state_home`. Reused. | `f7296ae3` |

### Not fixed

- **medium — file index is O(n²) for bulk changes in one folder** (search `index.rs` `child`/`insert`/`remove`, since `daa5d5e2`): siblings are a linked list walked per event, so extracting or deleting tens of thousands of files in one folder keeps the `file-index` thread busy for tens of seconds and launcher file queries wait. Not the UI thread. Needs a prev-sibling link and a per-folder name map for big folders; a data-structure change, not for tonight.
- **medium — shortcuts on non-Latin layouts and Shift+Ctrl+punctuation** (toolkit `seat.rs` `key_event`): Ctrl+C on a Russian layout gives a Cyrillic letter, and `Shift+Ctrl+,` (Terminal's Settings) and Terminal's `Shift+Ctrl+]`/`[` arrive as `<`, `}`, `{` and never match. The fix (level-1 keysym of the first Latin layout while Ctrl/Alt/Super is held) changes what Terminal's key encoder receives (Ctrl+Shift+2 → NUL needs `@`), so it needs Terminal changed with it.
- low — redrawable images (wallpaper, lock backdrop) redraw on the UI thread after an atlas reset or GPU fallback, and a failing redraw is retried every frame (`graphics/scene.rs:499`, `gpu.rs:432`).
- low — `Trash::dirs` stats `.Trash` paths on every mount (proc, sysfs, FUSE, autofs, network) on each call, and Files calls `is_empty` on its UI thread: a hung network mount blocks Files. Skip system mount types as GIO does.
- low — status hub: one `worker_lost` flag for all three workers, so only the first stopped worker is logged.
- low — packages `from_transaction` accepts transaction-path signals from any sender (path hard to guess); check PackageKit's unique name.
- low — volume trash in `fs::trash::dirs` does not require the sticky bit on `.Trash` as `TrashDir::for_volume` does (owner and link checks still apply).
- low — dead or test-only `pub` items: `fs::remove_all`, `fs::restore_to`, `thumbnails::MAX_QUEUED`, `search::format_number`, `datetime::WEEKS`, `audio::INTERNAL_PREFIX`, `network::ipv4_settings`, `network::vpn_type_of`, `TrashDir::for_volume`, `settings_panes::TEXT_SIZES`, `PaneHit::best`, `Spring::angular_frequency` (no caller), `pointer_speed_choices`, `devices::sound_of`, `battery_of`, `pixmap_rgba`, `without_mnemonics`, `history::charge_samples`, taskbar-model `app_id_from_uri`, `launcher_update`.
- low, docs (outside these paths) — `docs/design/features/windows.md:18,112,125` still list Super+M, Ctrl+Super+F, Super+W and Super+Q; `docs/architecture/behaviour.md:40` says Control-click opens the taskbar menu, which the toolkit no longer does.

Checked and sound: bande-app's runner (source mapping, quit, close/tab order, Settings window, activation and single instance, no idle wakeups), `bande_inbox`, the icon lookup cache, `write_launcher` quoting, the FIFO-safe MIME sniffing, `replace_file` users, the status hub relay, the BlueZ agent, app-menu exports (nothing left unused by bande-app), and no "unbound" names or unused dependencies.
