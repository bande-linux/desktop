# Userland code quality review

Date: 2026-10-31, at local `main` `2d497b1`. Scope: `crates/shell`, `crates/services`, `crates/apps`
and `crates/libs`: 47 crates, about 118 000 lines of Rust. The bar is a codebase fit for the
official Debian archive: no rot, maximal reuse, simple, efficient, best practice.

Method: a read-only review with no cargo build.
- Each category was swept across every file with grep.
- Scripts checked for unused dependencies and features, and for `pub` items that nothing uses or
  that only tests use.
- Every hit listed below was then read in context.

Line numbers are for `2d497b1`.

## What is already clean

- **Comments:** there are no code comments anywhere in scope.
- **Panics:** there is no `unwrap`, `expect`, `panic!`, `todo!` or `unreachable!` outside tests. All
  47 crates inherit `[lints] workspace = true`. The only lint escape is `#![allow]` inside a test
  module (`libs/xdg/src/launch.rs:420`).
- **No X11:** there are no X11 dependencies and no X11 code.
- **Dependencies:** there are only two unused dependencies (task 30).

## Findings by category

- **Backward-compatibility code** breaks the no-legacy rule. The config daemon still renames
  `panel`/`dock` to `bar` and migrates `output` nodes to `display` (task 3).
- **Duplicated helpers:**
  - 18 copies of `contains(Rect, Point)`.
  - About 25 hand-written XDG base-directory lookups. `unbound-xdg` has no `state_home` at all.
  - 16 copies of write-then-rename.
  - 10 hand-made clients of `org.unbound.Config`: 6 typed proxies and 4 that use property-name
    strings.
  - 5 definitions each of the UPower and login1 proxies.
  - 3 `NotificationCenter` proxies, 2 of them in Settings alone.
  - 6 hand-written wake pipes, while `unbound-inbox` already exists.
  - Two different types named `FolderWatch`.
  - 4 type-to-select implementations.
  - 5 readers of the user's passwd entry, one of which forks `getent`.
  - 2 "About this computer" fact readers, which give different answers.
  - An identical byte formatter in `unbound-fs` and Monitor.
  - 24 copies of the `L10N` boilerplate.
  - 9 mutex `lock` helpers, 5 `report` helpers, 3 `snap` helpers and 2 `percent_decode`.
- **Efficiency:**
  - Every bar (one per display) starts its own PipeWire connection, device follower, tray host,
    minute timer and config follower (task 1).
  - `find_icon` reads and parses `index.theme` on every call. When the size misses, it calls stat()
    on every candidate in every theme folder (task 2).
  - The shell parses the desktop entries and the MIME database again for each part.
  - Each launched app gets a fresh D-Bus connection and a thread that stays parked for the app's
    whole life (task 8).
  - Settings makes 41 property reads in a row on every `PropertiesChanged` signal (task 7).
- **Polling:**
  - The greeter wakes at 1 Hz and redraws every second to update a clock that shows minutes. The
    lock screen already uses `MinuteTimer`.
  - The thumbnailer and archive tools are waited on with 20 ms and 50 ms `try_wait` loops (task 13).
- **Shared connections:**
  - The desktop surface opens a session connection per display to follow the wallpaper, and another
    on every wallpaper choice.
  - The bar opens one for every battery-percentage toggle.
  - The lock screen connects to the session bus twice, one after the other, on its critical path.
  - Files and Terminal each hold two session connections (tasks 9 and 18).
- **Hidden panics:** 47 bare `std::thread::spawn` calls panic if the thread cannot be created, and
  their threads are unnamed (task 15).
- **Swallowed errors:**
  - There are 216 sites of `let _ =` or `.ok();`. Most of them ignore a send to a receiver that has
    gone away, which is fine.
  - Two clusters matter:
    - The 48 property-change emissions in `services/config/src/service.rs:147-304` (task 16).
    - The bar's errand sends: if the worker dies, menus stop opening without a trace
      (`shell/bar/src/menu_bar_client.rs:171-684`, part of task 1).
- **Layering:**
  - File Chooser depends on the Files app crate and on the portal service crate.
  - The lock screen, and so the greeter, links all of `unbound-bar` for `symbols.rs`.
  - Control Center links the power service.
  - `unbound-polkit` is a library that nothing uses and that has no binary (task 22).
- **Oversized modules:** 15 files are over 1000 lines. `shell/bar/src/dock.rs` holds a single
  1905-line `impl DockState` block (task 29).

## Ranked fix tasks

Each task fits one agent in 1–2 h, unless it says otherwise. "Saves" means approximate production
lines removed, net. Risk means behaviour risk once the gate passes. The order is value per hour,
with correctness and efficiency first.

### 1. One status hub for the shell, not one per display

- **Files:**
  - `shell/bar/src/menu_bar_client.rs:748-800`. Every `MenuBarClient::with_view` call starts:
    - `follow_devices`, which watches the system bus with 4 threads;
    - `follow_sound`, which makes a PipeWire `Audio::connect` (`libs/status/src/devices.rs:446-473`);
    - a minute-timer thread;
    - the tray `host` (`libs/status/src/tray.rs:178`), which registers the same
      `StatusNotifierHost-<pid>` name again and adds the match rules again;
    - `follow_unread`;
    - `follow_config`;
    - the errand worker.
  - `shell/bar/src/surfaces.rs:39-80` builds one of these per display, in both the rail and the
    menu-bar layout.
  - Control Center makes yet another `Audio::connect` at `shell/controlcenter/src/client.rs:897`.
- **Change:**
  - Add a `StatusHub` in `unbound-status`. It starts the followers once per process, keeps the last
    `Update` of each kind, and fans out to subscribed `Outbox<Update>`s; `Update` is already
    `Clone`.
  - The shell context owns the hub. Each bar subscribes to it and shares its control `Sender`s.
  - Control Center reuses the hub's `Audio`.
  - Log errand-worker death once instead of ignoring the sends.
- **Win:** with N displays, the shell drops (N−1)×(PipeWire client + about 8 threads + system-bus
  matches + tray host) and one more PipeWire client. A second display stops doubling wakeups on
  every battery, volume or tray signal.
- **Saves:** about 40 lines.
- **Risk:** medium, because of the start-up order of the bars. `shell/bar/tests/live.rs` covers the
  fan-out.

### 2. Cache icon-theme lookups in `unbound-xdg`

- **Files:** `libs/xdg/src/icon.rs`.
  - `theme()` (line 131) reads and parses `index.theme` for every theme in the inherit chain on
    every `find_icon` call (line 183).
  - When the size misses, `lookup_in` (138-176) calls stat() on every candidate in every theme
    folder, across every base folder and three extensions. That is hundreds of stat() calls for a
    miss in Adwaita, and again for hicolor.
- **Change:**
  - Add an `IconLookup` that keeps the parsed themes, and a per-folder `HashSet` of file stems
    filled lazily by one `read_dir`.
  - Keep `find_icon` as a thin wrapper over a process-wide `LazyLock<Mutex<IconLookup>>`, or pass
    the lookup in explicitly, whichever the 18 callers fit better.
  - Invalidate on a theme folder's mtime if the cache has to live long.
- **Win:**
  - Cold icon loads in Launcher, Files, Dock, Desktop and Monitor drop from O(folders × bases × 3)
    stat() calls plus a theme parse each, to one `read_dir` per folder ever touched.
  - Expect 10–100× fewer syscalls when Launcher first opens. Measure with `strace -c` before and
    after.
- **Saves:** about 0 lines.
- **Risk:** low. The `libs/xdg/tests/xdg.rs` icon tests stay.

### 3. Delete the config migration and legacy names

- **Files:**
  - `services/config/src/lib.rs`:
    - `RENAMED_SECTIONS`, `LEGACY_DISPLAY` and `section_name` (670-712);
    - `Config.outputs` (602, 641) and its fallback (701);
    - the legacy arms (754, 821, 1261).
  - `services/config/src/edit.rs`: `rename_sections` and `migrate` (298-387).
  - `services/config/src/service.rs`: `migrate_file` (67-80).
  - `services/config/src/main.rs`: the migrate call (23-27).
  - `services/config/src/connectors.rs`: the whole file, 39 lines, used only by the migration.
  - The exports at `lib.rs:19-27`.
  - Tests: `services/config/tests/service.rs:507-508`, the migration cases in `tests/edit.rs`, and
    `tests/config.rs:660`.
- **Change:** delete all of it. An unknown `panel`, `dock` or `output` node then gets the normal
  "unknown section" report.
- **Win:**
  - This follows the no-backward-compatibility rule.
  - The daemon no longer scans DRM in sysfs at start.
- **Saves:** about 230 production lines and about 120 test lines.
- **Risk:** low, since there are zero users.

### 4. Give `Rect` hit-testing and context-menu helpers to the toolkit

- **Files:** the same half-open `fn contains(Rect, Point)` appears 18 times:
  - `shell/bar/src/menubar.rs:200`, `dock.rs:275`, `menu_search.rs:246` and `rail.rs:106`;
  - `shell/controlcenter/src/modules.rs:42`, `client.rs:161` and `now_playing.rs:65`;
  - `shell/notifications/src/card.rs:430`;
  - `shell/launcher/src/rows.rs:31`;
  - `apps/settings/src/widgets.rs:33` and `arrangement.rs:224`;
  - `apps/terminal/src/app.rs:1871` and `tabs.rs:27`;
  - `apps/monitor/src/view.rs:68`;
  - `apps/files/src/view.rs:217`;
  - `toolkit/graphics/src/title_bar.rs:76` and `toolkit/widgets/src/lib.rs:343`.

  `opens_menu` appears 4 times: `shell/launcher/src/view.rs:680`, `apps/terminal/src/app.rs:1908`,
  `apps/monitor/src/view.rs:77` and `toolkit/widgets/src/lib.rs:421`. `is_secondary` is at
  `apps/monitor/src/view.rs:72`.
- **Change:**
  - Add `Rect::contains(self, Point)` in `toolkit/graphics/src/scene.rs`.
  - Make `unbound_widgets::opens_menu` and `Pointer::is_secondary` public.
  - Delete the copies, after checking that each body is the same half-open test.
- **Saves:** about 110 lines.
- **Risk:** very low.

### 5. Add a base-directory module to `unbound-xdg`, with `state_home`

- **Files:**
  - Move `config_home`, `data_home`, `cache_home` and `config_dirs` out of
    `libs/xdg/src/mimeapps.rs:14-45` into `libs/xdg/src/basedir.rs`.
  - Add `state_home()` and `unbound_state(name) -> Option<PathBuf>` (that is,
    `$XDG_STATE_HOME/unbound/<name>`).
  - Add `user_dirs_text()`, the read of `config_home/user-dirs.dirs` that is repeated 6 times, and
    `gtk_bookmarks_file()`, repeated twice.
- **Sites to replace:**
  - `shell/lock/src/backdrop.rs:43`
  - `shell/desktop/src/client.rs:870-873, 899`
  - `shell/desktop/src/places.rs:93-96`
  - `shell/screenshot/src/main.rs:144-152, 1064-1073`
  - `shell/greeter/src/main.rs:345`
  - `shell/launcher/src/client.rs:480-487`
  - `shell/shell/src/shell.rs:322-323`
  - `services/clipboard/src/main.rs:132-133` and `history.rs:24-29`
  - `services/notify/src/main.rs:9-10`
  - `services/config/src/main.rs:52`
  - `services/portal/src/settings.rs:173, 193`
  - `apps/terminal/src/main.rs:47` and `restore.rs:58-62`
  - `apps/file-chooser/src/main.rs:87-91, 162-175`
  - `apps/settings/src/login.rs:101`
  - `apps/files/src/main.rs:37-56, 401-442`, `views.rs:46-51` and `view.rs:207-215`
  - `apps/monitor/src/main.rs:203-209`
  - `libs/fs/src/trash.rs:288-292`
  - `libs/search/src/recents.rs:10-14` and `usage.rs:37-42`
  - `libs/xdg/src/autostart.rs:4-9`
- **Change:** one implementation that ignores relative paths, as the spec requires. Pure variants
  that take an `Option<&Path>` stay for tests.
- **Bugs fixed:**
  - `shell/screenshot/src/main.rs:149, 1067-1072` accepts a relative `XDG_*_HOME`.
  - Screenshot keeps its state and cache in `unbound-screenshot/` while everything else uses
    `unbound/`; the greeter uses `unbound-greeter/`.
- **Saves:** about 140 lines.
- **Risk:** low.

### 6. One typed `org.unbound.Config` client

Do this as two tasks: 6a covers the services and Settings, 6b covers the shell and the libraries.

- **Files:**
  - Typed partial proxies, plus duplicated `BUS_NAME`/`PATH` constants:
    - `services/sound/src/main.rs:12-28`
    - `services/power/src/adapter.rs:35-45` and `controller.rs:14-30`
    - `services/portal/src/settings.rs:34-35, 126-150`
    - `libs/wallpaper/src/choice.rs:11-12, 78-95`
    - `apps/settings/src/client.rs:11-200`
  - String-property clients:
    - `shell/lock/src/main.rs:184-193`
    - `shell/bar/src/menu_bar_client.rs:51, 214-216`
    - `libs/dock-model/src/bus.rs:15, 98-112, 161-182`
- **Change:**
  - Put one complete `ConfigProxy` with `gen_async = false` next to the interface, as
    `unbound_config::client`. This mirrors `unbound_notify::client` and `unbound_sound`.
  - Gate the daemon-only dependencies (`kdl`, `unbound-watch`, `unbound-notify`, `unbound-edid`)
    behind a default `service` feature, the same way `services/sound/Cargo.toml` does. Light
    consumers then use `default-features = false`.
- **Win:**
  - A misspelled property such as `"BarApps"` or `"ShowBatteryPercentage"` becomes a compile error
    instead of a runtime warning.
  - Each crate stops generating its own async and blocking proxy code.
- **Saves:** about 180 lines.
- **Risk:** low.

### 7. One property-following helper, reading changes from the signal

- **Files:** the same 15-line loop, which subscribes with `PropertiesProxy::receive_properties_changed`
  and then re-reads everything through an uncached proxy:
  - `apps/settings/src/follow.rs:141-170`, `time.rs:135-160`, `locale.rs:145-170` and
    `about.rs:215-235`
  - `libs/wallpaper/src/choice.rs:124-160`
  - `libs/dock-model/src/bus.rs:117-156`
  - `shell/bar/src/menu_bar_client.rs:831-850`
  - `services/portal/src/settings.rs:429-470`

  There are 16 `CacheProperties::No` proxies in all. `SettingsClient::read`
  (`apps/settings/src/client.rs:439-450`) makes 41 sequential `Get` calls for each signal.
- **Change:**
  - Add `unbound_bus::follow_properties(connection, destination, path, interface, FnMut(&HashMap<String, OwnedValue>) -> bool)`.
    It sends one `GetAll` first and then the `changed` dictionary of each signal.
  - Map the dictionary to updates, the way `libs/dock-model/src/bus.rs:33` already does for the
    launcher API.
- **Win:** a config change costs Settings one message instead of 41 round trips; it costs the
  wallpaper and dock followers zero extra calls.
- **Saves:** about 120 lines.
- **Risk:** medium, because properties can be missing from the dictionary. Keep a read when
  `invalidated` is not empty.

### 8. Stop opening a bus connection and parking a thread for each launched app

- **Files:** `libs/xdg/src/launch.rs`.
  - Lines 288-292: one thread per launch that has no app id.
  - Line 299: a new session connection when the spawn fails.
  - Line 303: `supervise` opens a new session connection for every launch to place the app in its
    systemd scope (lines 368-376), then blocks in `child.wait()` for the app's whole life.
  - Line 389: `tell` scans every desktop entry again just to name the app.
- **Change:**
  - `launch_with` takes the caller's `&Connection`; the shell, Files and Launcher already have one.
  - Add one reaper thread per process that waits on pidfds (`rustix::process::pidfd_open` + `poll`)
    for every supervised child.
  - Pass the entry's name in, instead of rescanning.
- **Win:** after 20 launches, the shell holds 1 thread instead of 20 parked ones, and there are no
  connection handshakes on the launch path. That is faster time-to-scope placement.
- **Saves:** about 20 lines.
- **Risk:** medium. Crash notifications are covered by `libs/xdg/tests`.

### 9. Let the shell share its connection, config and catalog with its parts

- **Files:**
  - `shell/desktop/src/client.rs:912` calls `follow()` once for each per-display surface. Each call
    opens its own session connection and thread (`libs/wallpaper/src/choice.rs:121-163`).
  - Every wallpaper choice opens another connection (`choice.rs:173-186`, called from
    `shell/desktop/src/client.rs:563`).
  - `shell/bar/src/menu_bar_client.rs:212-222` connects once for each toggle.
  - `shell/bar/src/dialogs.rs:141` connects to the system bus again; `Services` already holds it.
  - `shell/shell/src/shell.rs:39-45` re-reads and re-parses `config.kdl` from disk for every part it
    opens.
  - Desktop entries are parsed again by:
    - `shell/notifications/src/client.rs:325`
    - `shell/bar/src/dialogs.rs:32, 371`
    - `shell/bar/src/dock_client.rs:1012`, per display
    - `shell/bar/src/menu_bar_client.rs:755`, per display
    - `shell/desktop/src/client.rs:452`, on every "Change Wallpaper"
    - `shell/launcher/src/client.rs:469`
    - `shell/controlcenter/src/client.rs:482`
  - `MimeDatabase` is loaded by `shell/launcher/src/client.rs:105` and again through `Apps::load`
    at 495, and once per desktop surface (`shell/desktop/src/client.rs:920`).
- **Change:**
  - `Context` holds an `Rc` catalog (entries, `Apps`, `MimeDatabase`) and the followed `Config`,
    and passes `&Connection` to `wallpaper::follow` and `wallpaper::choose`.
  - Parts stop calling `applications()` themselves.
- **Win:**
  - Shell start-up does one desktop-entry scan and one shared-mime-info parse instead of 6 + 2N
    scans and 2 + N parses.
  - Several hundred KB less memory per extra parse.
  - N fewer connections and threads.
- **Saves:** about 60 lines.
- **Risk:** medium. This may take more than 2 h; if it does, split the catalog half from the
  connection half.

### 10. Use `unbound_inbox::Inbox` instead of hand-written wake pipes

- **Files:**
  - `apps/settings/src/follow.rs:95-133` reimplements `Inbox`/`Outbox` completely, as
    `Feed`/`Updates`.
  - `apps/files/src/loader.rs:67-100` and `libs/fs/src/ops.rs:127-150` use a channel plus a
    `UnixStream` pair.
  - `apps/terminal/src/app.rs:117-147`: `SettingsWatch`.
  - `services/clipboard/src/main.rs:115`.
  - `shell/lock/src/main.rs:208`: the PAM verdicts.
- **Change:** use `Inbox<T>` and `Outbox<T>`. Add the `unbound-inbox` dependency to Settings,
  Clipboard and `unbound-fs`.
- **Saves:** about 110 lines.
- **Risk:** low.

### 11. Give the toolkit one icon image cache that follows the accent

- **Files:**
  - `apps/files/src/icons.rs`, the whole file.
  - `apps/file-chooser/src/view.rs:357-400`.
  - `apps/monitor/src/view.rs:45-66` and `main.rs:380-386`.
  - Ad hoc maps in `shell/launcher/src/client.rs:40-50, 420-426`, `shell/desktop/src/client.rs:160-166`,
    `shell/bar/src/dock_client.rs:113-122`, `shell/bar/src/rail.rs:443-453, 552-558`,
    `shell/controlcenter/src/client.rs:97-110` and `apps/settings/src/header.rs:28-34`.
  - `shell/bar/src/menu_bar_client.rs:335-342` resolves and rasterizes the focused app's icon with
    no cache, on every focus change.
- **Bug:** only File Chooser follows the accent icon theme
  (`services/portal/src/settings.rs:87 icon_theme`). Every other app uses the fixed
  `ICON_THEMES = ["Unbound", "Adwaita"]` (`libs/xdg/src/icon.rs:5`), so folder colours differ
  between File Chooser and Files.
- **Change:**
  - Add `unbound_toolkit::Icons` with `first(names, px)`, `symbol(names, px, color)` and
    `set_accent`.
  - Move `icon_theme(accent)` into `unbound-xdg`.
- **Saves:** about 150 lines.
- **Risk:** low to medium, because goldens can change.

### 12. Remove the duplicate `NotificationCenter` proxies

- **Files:**
  - `apps/settings/src/client.rs:201-212` and `apps/settings/src/notifications.rs:18-31` duplicate
    `services/notify/src/client.rs:54-84`. Settings already depends on `unbound-notify`.
- **Change:** add `get_apps` and `set_app_rule` to `unbound_notify::client::NotificationCenter`, and
  delete both copies.
- **Saves:** about 30 lines.
- **Risk:** very low. This takes 30 min.

### 13. Replace polling with timers and pidfds

- **Files:**
  - `shell/greeter/src/main.rs:25, 314-330`. A `CLOCK_TICK` of 1 s plus an unconditional
    `request_redraw` repaints the login screen every second for an HH:MM clock. Use
    `unbound_datetime::MinuteTimer`, as `shell/lock/src/main.rs:37, 225` does.
  - `libs/thumbnails/src/generate.rs:101-112` polls `try_wait` every 20 ms for up to 15 s, and
    `libs/fs/src/ops/archive.rs:420-440` polls every 50 ms. Add one shared
    `wait_child(child, deadline, cancelled)` on `pidfd_open` + `poll`. It could live in
    `unbound-inbox`, next to the threading helpers.
- **Win:** an idle greeter wakes 60× less and stops repainting. Thumbnailer waits stop costing
  50 wakeups a second each.
- **Saves:** about 10 lines.
- **Risk:** low.

### 14. One write-then-rename helper, with fsync

- **Files:** there are 16 copies:
  - `shell/desktop/src/places.rs:100-106`
  - `shell/screenshot/src/lib.rs:100-105`
  - `services/config/src/service.rs:61-65` and `login_picture.rs:50-58`
  - `services/portal/src/gtk.rs:74-79`
  - `services/clipboard/src/history.rs:236-246`
  - `apps/monitor/src/main.rs:212-221`
  - `apps/files/src/main.rs:62-69` and `views.rs:130-135`
  - `apps/file-chooser/src/main.rs:98-116`
  - `apps/terminal/src/restore.rs:48-53` and `settings.rs:136-141`
  - `libs/search/src/usage.rs:105-112`
  - `libs/xdg/src/mimeapps.rs:232-239`
  - `libs/wallpaper/src/cache.rs:45-53`
  - `libs/thumbnails/src/cache.rs:190-199`
- **Change:** add `unbound_xdg::replace_file(path, bytes)`. It creates the parent, writes to a
  sibling staging file, calls `sync_all` and renames. Only `login_picture.rs:56` syncs today, so a
  power loss can leave `config.kdl` empty on some filesystems.
- **Saves:** about 90 lines.
- **Risk:** low.

### 15. Name threads and stop panicking when a spawn fails

- **Files:**
  - There are 47 bare `std::thread::spawn` calls in scope. Examples are
    `shell/bar/src/menu_bar_client.rs:213, 764-790`, `libs/status/src/devices.rs:388-472`,
    `services/portal/src/settings.rs:459-470` and `libs/xdg/src/launch.rs:291-303`.
  - `std::thread::spawn` panics if the OS refuses the thread, which breaks the no-panic rule, and
    its threads show up unnamed in `top -H`.
  - The logging spawn helper exists twice: `apps/settings/src/follow.rs:135` and
    `libs/dock-model/src/bus.rs:54`.
- **Change:** add `unbound_inbox::spawn(name, work) -> bool` and replace all the sites.
- **Saves:** about 0 lines.
- **Risk:** very low.

### 16. Emit config property changes from a table

- **Files:** `services/config/src/service.rs:147-304` makes 48 hand-written
  `if before.x != after.x { let _ = guard.x_changed(emitter).await; }` checks and drops every error.
- **Change:** build the list of changed property names from a `(name, fn(&Config) -> OwnedValue)`
  table. Emit a single `org.freedesktop.DBus.Properties.PropertiesChanged` with
  `zbus::fdo::Properties::properties_changed`, and log a failure once.
- **Win:** one signal for each reload instead of up to 48, so clients wake once.
- **Saves:** about 110 lines.
- **Risk:** low to medium. Clients that listen to individual property streams still receive the
  standard signal.

### 17. Share the system-service proxies and stop generating async code

- **Files:**
  - UPower and UPower.Device are defined in:
    - `services/power/src/battery.rs:26-104`
    - `services/power/src/adapter.rs:13-21`, which repeats UPower inside the same crate
    - `apps/settings/src/battery.rs:18-64`
    - `libs/status/src/devices.rs:32-52`
  - `login1.Manager` is defined in `services/power/src/logind.rs:12`, `apps/settings/src/software.rs:33`,
    `apps/monitor/src/inhibitors.rs:9` and `libs/status/src/devices.rs:59`.
  - `PowerProfiles` is defined twice. `bluez.Device1` is defined twice in `unbound-bluetooth`
    (`lib.rs:28`, `agent.rs:40`).
  - 37 of the 46 `#[zbus::proxy]` blocks in scope generate async proxies that nothing uses.
- **Change:**
  - Move the UPower, login1 and PowerProfiles proxies into `unbound-bus` (`upower`, `login1`),
    blocking only.
  - Merge the bluez `Device1`.
  - Add `gen_async = false` wherever no `.await` is involved.
- **Win:** less generated code, and shorter compile and link times for 10 crates.
- **Saves:** about 120 lines.
- **Risk:** low.

### 18. Make each process use one session connection

- **Files:**
  - Files: `AppMenu::export` (`apps/files/src/main.rs:454`) plus a second connection at
    `main.rs:347`.
  - Terminal: `apps/terminal/src/main.rs:97` plus `bus.rs:47`.
  - Lock: `shell/lock/src/main.rs:185` (`clock_format`), then `backdrop.rs:34` connects again while
    the screen is being locked.
  - Screenshot: `service.rs:455`, `main.rs:1114` and `indicator.rs:94`.
- **Change:** open the connection once in `main` and use `AppMenu::export_on`; Settings already
  does this at `apps/settings/src/main.rs:244`.
- **Win:** fewer handshakes on the lock path.
- **Saves:** about 15 lines.
- **Risk:** low.

### 19. Merge the two `FolderWatch` types

- **Files:** `libs/fs/src/watch.rs` is fd-based and fits the event loop; it is used by the dock,
  Desktop and Files. `libs/watch/src/lib.rs` is a thread plus a callback; it is used by the config
  daemon, search and Terminal. The two types share a name and work differently.
- **Change:**
  - Keep the fd-based type in `unbound-watch`.
  - Make `spawn(changes, callback)` a small adapter over it.
  - `unbound-fs` re-exports or depends on it.
  - Terminal can then register the fd directly and drop its wake pair.
- **Saves:** about 80 lines.
- **Risk:** low.

### 20. One source of "About this computer" facts

- **Files:**
  - The bar's About dialog uses `shell/bar/src/dialogs.rs:270-277` and
    `libs/status/src/session.rs:205-250`: chip from `cpuinfo`, memory rounded to GiB.
  - Settings › About uses `apps/settings/src/about.rs:55-200`: chip from device-tree `compatible`,
    memory rounded up to GB×1e9.
- **Bug:** the two can show a different chip and memory size for the same Mac.
- **Change:** move one `Facts` reader into `unbound-status` and use it in both places.
- **Saves:** about 50 lines.
- **Risk:** low, but goldens will change.

### 21. One passwd reader

- **Files:**
  - `shell/lock/src/user.rs:21-65`, which calls `getpwuid_r` through `libc` in an unsafe block.
  - `shell/greeter/src/users.rs:5-25` and `main.rs:343`.
  - `libs/status/src/session.rs:254-267` (`full_name`).
  - `apps/monitor/src/proc.rs:140`.
  - `services/polkit/src/agent.rs:138-149`, which forks `getent`.
- **Change:** add `crates/libs/users`, holding passwd parsing, `display_name(login, gecos)` and the
  current user. The greeter then drops `libc`; it uses it only for `O_NOFOLLOW`
  (`accounts.rs:14`), which rustix has.
- **Saves:** about 60 lines.
- **Risk:** low.

### 22. Decide what happens to `unbound-polkit`

- **Files:** `crates/services/polkit`: 440 production lines and 444 test lines.
  - It has no binary, and no crate depends on it.
  - `docs/architecture/session.md:252-264` says it waits for a window.
  - Settings asks polkit for interaction (`apps/settings/src/privilege.rs`), but no agent runs in
    the session, so privileged actions that need a password fail.
- **Change:** either wire the agent into the shell with a toolkit sheet, or delete the crate and its
  CI job (`.github/workflows/ci.yml:383-400`, `ci/polkit`) until it is built.
- **Risk:** this is a product decision for the owner.

### 23. Fix the layering

Do this as three tasks.

- **a.** File Chooser depends on the Files app crate (`apps/file-chooser/Cargo.toml`) only for the
  sidebar model: `unbound_files::{rows, sidebar_file, with_volumes, Place, Row, Location}`. It also
  depends on the portal service for `dialog_process` and the `file_chooser` types. Move
  `apps/files/src/sidebar.rs` into a library (for example `unbound-fs::places`), and move the dialog
  wire protocol into a small library shared by the portal and File Chooser.
- **b.** `shell/lock/src/lib.rs:11` links all of `unbound-bar`, and through it the greeter does too,
  for `symbols.rs`. Move `shell/bar/src/symbols.rs` (247 lines) to `toolkit/widgets`.
- **c.** Control Center links the power service for `backlight` and `logind::own_session`
  (`shell/controlcenter/src/client.rs:20-21`). Move both into a library.

  Also review `libs/xdg` → `unbound-notify` (launch-failure notices) and `libs/dock-model` →
  `unbound-config`. Task 6 settles the second.
- **Risk:** low. These are moves only.

### 24. Make localization boilerplate one macro

- **Files:**
  - 24 copies of `const ENGLISH = include_str!(…); static L10N: LazyLock<Localizer> = …`, for
    example `apps/settings/src/lib.rs:69-71` and `libs/xdg/src/lib.rs:14-18`.
  - The coverage tests list source files by hand (`apps/settings/src/lib.rs:76-100`), so a new file
    is silently left uncovered.
- **Change:**
  - Add `unbound_l10n::localizer!()`, which builds the path from `CARGO_MANIFEST_DIR` and
    `CARGO_PKG_NAME`.
  - Add a `missing_ids_in_crate()` test helper that walks `src/` at run time.
  - A second language then becomes a change in one place, which matters for Debian translators.
- **Saves:** about 60 lines.
- **Risk:** low.

### 25. One type-to-select helper

- **Files:**
  - `shell/desktop/src/view.rs:30, 1008-1020`
  - `apps/files/src/view.rs:46, 201, 1257-1270`
  - `toolkit/widgets/src/table.rs:26, 807-820`
  - `toolkit/widgets/src/menu.rs:37, 1208-1225`
- **Change:** add `unbound_widgets::TypeSelect { typed, at }` with `push(char, now) -> &str` and
  `deadline()`.
- **Saves:** about 50 lines.
- **Risk:** low.

### 26. Settings: use the toolkit `ScrollView` and draw a real check mark

- **Files:**
  - `apps/settings/src/widgets.rs:813-935` is a private `Scroll` without the toolkit's momentum or
    rubber band (`toolkit/widgets/src/scroll.rs:394`).
  - `widgets.rs:1356-1420` draws `Check` by stamping 18 dots.
- **Change:** use `ScrollView` and keep focus-follow as an option of it. Stroke a `Shape` path for
  the check.
- **Saves:** about 150 lines.
- **Risk:** medium, because goldens change and scrolling feels different. That change is the point.

### 27. One status-glyph system

- **Files:**
  - The menu bar and the lock screen draw battery and Wi-Fi from `shell/bar/src/symbols.rs`.
  - The combined bar uses icon-theme names (`shell/bar/src/rail.rs:117-165`, with its own `snap`
    at 113).
  - Control Center uses theme glyphs (`shell/controlcenter/src/modules.rs:20-30`).
  - So the same battery appears in three styles.
- **Change:** pick one style, following Datum's "rich bar icons", and delete the other.
- **Risk:** design sign-off is needed.

### 28. Small duplicate helpers

Do these as one batch.

- **`report(what, result)`:** `shell/bar/src/menu_bar_client.rs:79`, `shell/bar/src/dialogs.rs:25`,
  `shell/notifications/src/client.rs:53`, `shell/controlcenter/src/client.rs:72` and
  `libs/status/src/devices.rs:198`. Move it to `unbound_log::report`.
- **Mutex `lock` helpers:**
  - `shell/screenshot/src/service.rs:103`
  - `services/clipboard/src/bus.rs:21`
  - `services/portal/src/screencast.rs:62`
  - `services/polkit/src/agent.rs:179`
  - `services/config/src/service.rs:86`
  - `services/tray/src/lib.rs:28`
  - `services/automount/src/lib.rs:73`
  - `services/notify/src/lib.rs:127`
  - `services/sound/src/main.rs:36`

  Make one generic `unbound_bus::lock`.
- **`snap`:** `shell/bar/src/rail.rs:113`, `shell/bar/src/symbols.rs:142` and
  `apps/terminal/src/render.rs:33`. Move it to `unbound-graphics`.
- **`percent_decode`:** `apps/terminal/src/process.rs:151` duplicates `unbound_xdg::percent_decode`
  (`libs/xdg/src/uri.rs:25`).
- **Byte formatting:** `apps/monitor/src/format.rs:3-27` has the same body as
  `libs/fs/src/listing.rs:159-183`, with duplicate FTL strings. Move it to
  `unbound_l10n::format_bytes`.
- **Desktop › Change Wallpaper** (`shell/desktop/src/client.rs:452`) rescans every entry to find
  Settings, although `self.apps` is already loaded.
- **Saves:** about 90 lines.
- **Risk:** very low.

### 29. Split oversized modules

There is no behaviour change. Do one task per file.

- **`shell/bar/src/dock.rs` (2555 lines):** the `impl DockState` block runs from 301 to 2206.
  Split it into `dock/`:
  - `layout.rs`: layout, icon_rect, along and reach;
  - `drag.rs`: Drag, Ghost and lines 664-893;
  - `hiding.rs`: lines 347-456;
  - `navigation.rs`: lines 503-603;
  - `paint.rs`: lines 1372-1900, plus Tile, MoreTile and Dock.
- **`apps/settings/src/widgets.rs` (2038 lines):** 20 widgets. Split them into `layout.rs` (Root,
  Column, Card, Row, Captioned), `sidebar.rs`, `pictures.rs` (Thumbnails, Swatches) and `meters.rs`
  (UsageBar, LevelHistory, Signal).
- **`apps/terminal/src/app.rs` (1955 lines):** move tabs, scrolling with `Momentum`, the settings
  watch and `Arguments` out of the 1375-line `impl TerminalApp`.
- **`services/config/src/lib.rs` (1456 lines):** move `Parser` (733-1437) into `parse.rs`.
- **Others:** `apps/files/src/view.rs` (1892), `apps/settings/src/view.rs` (1604),
  `apps/monitor/src/model.rs` (1532), `shell/desktop/src/view.rs` (1484),
  `apps/monitor/src/monitor_view.rs` (1393) and `shell/bar/src/rail.rs` (1264).
- **Risk:** very low.

### 30. Tidy manifests and names

- **Unused dependencies:** `shell/greeter/Cargo.toml:25` has an unused dev-dependency, `rustix`.
  `services/clipboard/Cargo.toml` lists `wayland-backend`, which no source file uses; check whether
  it is only there to unify a feature, then drop it.
- **Unsorted `[workspace.dependencies]`:** the repository-layout rule asks for sorted entries, but:
  - `alacritty_terminal` comes before `accesskit`;
  - `ab_glyph` comes after `accesskit_unix`;
  - `image` comes between `bzip2` and `flate2`;
  - `md-5` comes before `kdl`;
  - `serde` comes before `ruzstd`;
  - the block from `naga` to `wgpu` is unordered;
  - `unbound-datetime` comes before `unbound-controlcenter`.
- **Package names that differ from their folders:** `unbound-term` lives in `apps/terminal`, and
  `unbound-desktop-surface` lives in `shell/desktop`.
- **Bar module names:** `menubar.rs` sits next to `menu_bar_client.rs`.
- **Saves:** about 0 lines.
- **Risk:** very low.

### 31. Delete dead code and API that only tests use

Do these as one batch.

- **Dead:**
  - `BarEdge::is_vertical` (`services/config/src/lib.rs:119`)
  - `unbound_sound::THEME_NAME` (`services/sound/src/lib.rs:13`)
- **Used only by their own tests:**
  - `Idle::preparing_for_sleep` (`services/power/src/idle.rs:126`). The lock before sleep goes
    through `logind.rs:138-155` instead.
  - `Spring::from_duration_bounce`, `Animation::retarget_with_velocity` and `Decay::FAST`
    (`libs/spring/src/lib.rs:62, 164, 189`)
  - `NotificationServer::invoke_action` (`services/notify/src/lib.rs:298`)
  - `History::next_undo` and `next_redo` (`libs/fs/src/undo.rs:36-42`). Either delete them, or wire
    them into the Edit menu so it shows "Undo Move" the way macOS does; the menu says only "Undo"
    (`apps/files/src/menu_bar.rs:44`).
- **Saves:** about 60 lines, plus their tests.
- **Risk:** very low.

### 32. One wall-clock timer

- **Files:**
  - `libs/datetime/src/minute.rs`, which is `MinuteTimer`.
  - `services/portal/src/settings.rs:383-470`, a hand-written absolute wall-clock timerfd plus three
    threads (a timer follower, a signal forwarder and a worker).
  - `services/notify/src/alarm.rs`, which uses the boot-time clock plus an eventfd. That is a
    different clock, so keep it.
- **Change:**
  - Generalize `unbound_datetime::WallClockTimer::at(Timestamp)` and build `MinuteTimer` on it.
  - The portal polls the timer fd and the bus in one thread.
- **Saves:** about 40 lines and two threads.
- **Risk:** low.

## Top 10

1. **One status hub per shell process** (task 1). Today every display's bar runs its own PipeWire
   client, device follower, tray host and about 8 threads.
2. **Cache icon-theme lookups in `unbound-xdg`** (task 2). Every `find_icon` call parses
   `index.theme` again and can make hundreds of stat() calls.
3. **Delete the config migration and legacy section names** (task 3). This follows the no-legacy
   rule and removes about 350 lines.
4. **`Rect::contains`, `opens_menu` and `is_secondary` in the toolkit** (task 4). This removes 18
   plus 4 copies.
5. **A base-directory module in `unbound-xdg`, with `state_home`** (task 5). This removes about 25
   hand-written lookups and fixes Screenshot's spec bug and path layout.
6. **One typed `org.unbound.Config` client** (task 6). This replaces 10 hand-made clients and makes
   property names compile-checked.
7. **One `follow_properties` helper that reads `GetAll` and the signal's changes** (task 7). Settings
   makes 41 round trips per change today.
8. **No per-launch bus connection, and no thread parked per running app** (task 8).
9. **The shell shares its connection, config and catalog with its parts** (task 9). This ends the
   per-display wallpaper followers, the re-parsed `config.kdl`, and up to 6 + 2N desktop-entry
   scans.
10. **`Inbox` instead of 6 hand-written wake pipes** (task 10). This includes the complete copy in
    Settings' `follow.rs`.

Quick wins to run alongside the top 10, each 30 min or less:
- task 12, the duplicate `NotificationCenter` proxies;
- task 31, dead code;
- task 30, manifests;
- task 13, the greeter's 1 Hz redraw.
