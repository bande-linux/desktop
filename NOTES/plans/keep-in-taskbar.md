# Keep in Taskbar, and renaming the Dock to the Taskbar

## Summary

Keep in Dock fails for two reasons. The main one, and the one behind VS Code, Zed and Claude on this mini: the bar reads desktop entries only once, when the shell starts. The other: an app that matches no entry at all can never really be kept. The fix is to reload entries when apps are installed (already planned in `taskbar-icons.md`), then match a window to an entry through its process, then write a launcher for apps that still have none. The rename only touches UI strings, code names and docs. Config and D-Bus already say `bar`, so no stored setting has to move.

## What the owner sees

- You start `code` from Terminal, right-click its tile and choose Keep in Dock. The tick appears and the id is saved (`~/.config/bande/config.kdl` lists `com.microsoft.VSCode`, `dev.zed.Zed` and `com.anthropic.Claude`). But the tile is named "VSCode" with a letter icon, and when you quit VS Code it vanishes from the dock. Clicking it would not start the app anyway.
- An app with no desktop file (a dev build, an AppImage, a script) shows the same thing: Keep looks like it worked, and the tile disappears when the app quits.
- The bar says "Dock" in "Keep in Dock", in the screen reader label and in a few Settings strings. Settings calls the whole thing "Bar".

## Findings

1. **Entries are loaded once (root cause here).** `Dock::new` calls `applications()` a single time (`crates/shell/bar/src/dock_client.rs:1012`), and nothing reloads it. `bande-shell` (pid 66932) has run since 07:52 today. Zed's entry was written at 09:59, Claude was installed at 10:01 and VS Code at 10:08 (`/var/log/dpkg.log`, `apt/history.log`, `ps -o lstart`). So none of the three has an entry in the running bar. The `taskbar-icons.md` planner found the same thing independently.
2. **A kept app with no loaded entry is dropped.** `describe` returns `None` when `kept && entry.is_none() && !running` (`dock_client.rs:155`). Keep stores whatever `identity()` returns (`dock_client.rs:127`, `:564-571`), which is the raw window app_id when `find_app` fails. The id is saved, but the tile only shows while the app runs. `arrange` keeps such ids in the list without showing them (`crates/libs/dock-model/src/lib.rs:40`). `open` returns early without an entry (`dock_client.rs:236`).
3. **VS Code's app_id matching is fine once entries are loaded.** VS Code 1.144 renames its desktop name at runtime to `com.microsoft.VSCode` when `/usr/share/applications/com.microsoft.VSCode.desktop` exists (`/usr/share/code/resources/app/out/mainImpl.js`, `setDesktopName`; `product.json` `linuxDesktopName`). So Electron sets the window app_id to `com.microsoft.VSCode`, launched from Terminal or not. `~/.local/state/bande/windows` shows `com.microsoft.VSCode` for the main window, plus `code` and `claude-desktop` for the apps' GTK side dialogs (GTK uses the program name). `find_app` (`crates/libs/xdg/src/identity.rs:25`) matches all of these: exact id first, then the Exec program rule (`/usr/share/code/code` gives `code`; `claude-desktop`). Matching is not the cause on this machine.
4. **Nothing beyond app_id is used.** The bar only sees app_ids from `ext_foreign_toplevel_list_v1` (`crates/toolkit/platform/src/shell.rs:206`), with no pid. The compositor knows each client's pid (`client.get_credentials`, `crates/platform/compositor/src/dock.rs:98`) but never sends it for this. GNOME falls back to the window's pid and startup sequence, and finally a window-backed app [1]. KDE falls back to `/proc/<pid>/environ` hints and then matches the process command line against every `Exec` [2].
5. **The cgroup alone is not safe.** VS Code's main process moves itself into `app-com.microsoft.VSCode-206395.scope`, and 206395 is its own pid. Its zygotes stay in `app-bande-org.bande.Terminal-188855.scope`. Anything started from Terminal inherits Terminal's scope, so reading the scope would claim a GTK app run from Terminal *is* Terminal. A scope can only be trusted when its trailing pid is the window's pid, or when it is a Flatpak scope (`app-flatpak-<id>-<n>.scope`). `unit_app_ids` (`crates/libs/xdg/src/units.rs:39`) already parses these names. `/proc/<pid>/environ` is useless for Chromium apps: VS Code's reads empty (setproctitle overwrites it).
6. **Rename inventory.** The config key is already `bar { apps … }` and the D-Bus properties are already `BarApps`, `BarEdge` and so on (`crates/libs/dock-model/src/bus.rs:15-105`). There is no `bande-dock` package. Shortcuts are not stored in config. What still says Dock:
   - User-facing strings: `menu-keep = Keep in Dock` and `dock = Dock` (the accessible label, `crates/shell/bar/src/dock.rs:2516`) in `crates/shell/bar/locales/en-US/bande-bar.ftl:51,65`; `org.bande.Settings.desktop:5,9`; the `bar-*-keywords` lines in `bande-settings-panes.ftl`.
   - Code names: crate `bande-dock-model` (`crates/libs/dock-model`, `Cargo.toml:86`); `crates/shell/bar/src/dock.rs` and `dock_client.rs` (`Dock`, `DockState`, `DockApp`, `DockAction`, `dock_role`, `dock_ui`); `tests/dock.rs` and the goldens `dock-{left,bottom}-light.png`; the layer namespace `"dock"` (`compositor/src/motion.rs:183`, `displays.rs:14`, `input.rs:164,950`, `popup.rs:203`, `dock.rs:171`, `bar/src/dock_client.rs:41`); compositor `src/dock.rs`, which holds app operations, and `tests/headless/dock.rs`; `Action::FocusDock` and `focus-dock` (`crates/libs/keyboard/src/shortcuts.rs:69,151`); the protocol descriptions in `bande-shell-v1.xml`; the log filter test in `crates/libs/log/src/lib.rs:101`.
   - Docs: `docs/design/features/dock.md` (already marked superseded by `bar.md`), plus about 25 docs and `ci/review/scenarios`. The `images`, `system-scripts` and `apt` repos only mention the `apple-dockchannel` driver, so nothing to change there.

## Options

**Matching**
- A. **Reload entries only** (`taskbar-icons.md` milestone 1). Fixes VS Code, Zed and Claude here. Apps with no entry still can't be kept.
- B. **A + match by process.** The compositor sends each app's pid. The bar tries a trusted scope, then `/proc/<pid>/exe` against each entry's resolved Exec program. This catches apps whose app_id matches no entry but whose binary has one (old Electron builds, renamed classes, Flatpak). Cost: one protocol event and one xdg function.
- C. **B + write a launcher when nothing matches.** Keep writes `~/.local/share/applications/<app_id>.desktop` from the process: `Exec` is the canonical exe, or interpreter plus script; `TryExec` is the exe; `StartupWMClass` is the app_id; `Icon` is the app_id if the theme has one. Then every running app can be kept, launched, opened at login and shown in Files. It works like Windows pinning an exe [3, from knowledge]. The cost is a user-visible file, and the app then also shows in the launcher.
- D. **B + keep without a launcher**: store the command in `config.kdl`. This needs a parallel launch path, and the launcher, Open at Login and Show in Files would all miss it. Rejected.

**Name**
- "Taskbar" for the whole combined bar (launcher, apps, tray, clock; this is the Windows sense) and for the side strip in the separate-menu-bar layout. Settings ▸ Bar becomes Settings ▸ Taskbar, the menu item becomes Keep in Taskbar.
- Or "Taskbar" only for the apps area, with "Bar" kept as the whole. Two names for one strip is confusing.

## Recommendation

Do C on top of the reload from `taskbar-icons.md`. Call the whole bar "Taskbar" in the UI. Rename the Dock code to `taskbar`, but keep the crate `bande-bar` (it holds both the taskbar and the menu bar), the config key `bar` and the `Bar*` D-Bus properties, so nothing stored has to migrate. Do the internal rename last, once no other branch is editing `crates/shell/bar`.

## Implementation plan

Progress (2026-11-01): milestones 1-5 and 7 done (the bar keeps an app with no entry by writing a
launcher; the interface and docs say Taskbar). Milestone 6, the code rename, is left: other
branches were still editing `crates/shell/bar` and the compositor. The bar-level `keep.rs` test of
milestone 4 is not written; `bande-xdg` tests the written launcher.

0. **Dependency: the bar reloads desktop entries** (`taskbar-icons.md` milestone 1, owned there). Land it first. On its own it fixes the owner's VS Code case. When it lands, also reload in `menu_bar_client.rs:759`, so the active app's name is right too. Suggest that plan put the watcher in `bande_xdg` so the bar, menu bar and launcher share it.

1. **The compositor tells the shell each app's process** (~1.5 h)
   - `crates/platform/protocols/protocols/bande-shell-v1.xml`: new event `app_process(app_id: string, pid: int)`. It is sent when a toplevel first gets an app_id, or changes it, and on bind for every mapped app. Protocols are at version 1 with no version checks (f09a7874), so it is just added.
   - Compositor: send it next to the foreign-toplevel update (`crates/platform/compositor/src/shell.rs:332`) and in the bind path, using `get_credentials` as `force_quit_app` already does (`dock.rs:98`).
   - `crates/toolkit/platform/src/shell.rs`: store the pids by app_id and add `Window::app_pid(app_id) -> Option<u32>`.
   - Test: `crates/platform/compositor/tests/headless/shell.rs`. A test client maps a toplevel with app_id `org.example.Stray`, and the shell client receives `app_process("org.example.Stray", std::process::id())`. Changing the app_id sends the new id.

2. **Find an app's entry from its process** (~1.5 h)
   - `crates/libs/xdg/src/identity.rs`: add `find_app_for_process<'a>(entries, proc_root: &Path, pid) -> Option<&'a DesktopEntry>`, taking a root as `app_slice` does (`crates/libs/status/src/memory.rs:78`).
     - (a) Read `<root>/<pid>/cgroup`. Accept the unit when it is a Flatpak scope, or when its trailing number is `pid`. Then run `find_app` on each id from `unit_app_ids`.
     - (b) Canonicalize `<root>/<pid>/exe`, dropping a trailing ` (deleted)`. Compare it with each entry's first `command()` word, resolved through `find_executable` and canonicalized. Skip `Terminal=true` entries and the `LAUNCHERS` interpreters.
   - Tests in `crates/libs/xdg/tests/xdg.rs`, with a temp proc root holding `cgroup` files and `exe` symlinks:
     - pid 42 in `app-com.example.Editor-42.scope` resolves to that entry;
     - a process in `app-bande-org.bande.Terminal-7.scope` with pid 42 does *not* resolve to Terminal, but its exe does match `Exec=/abs/path/editor`;
     - an exe of `python3` matches no entry whose Exec is `python3 tool.py`;
     - a Flatpak scope resolves by its id.

3. **The bar falls back to the process** (~1.5 h)
   - `crates/shell/bar/src/dock_client.rs`:
     - `follow_windows` builds `(window_app, identity)` pairs. When `find_app` fails, it uses `find_app_for_process(&entries, "/proc", window.app_pid(..))`.
     - `window_app()` looks the pair up instead of calling `running_app`.
     - `identity()` and `describe()` read the pairs.
   - Keep `running_app` in `crates/libs/xdg/src/identity.rs`, because `crates/shell/controlcenter/src/client.rs:484` still uses it.
   - Test: new `crates/shell/bar/tests/keep.rs`. Copy the harness setup from `tests/live.rs` but use its own env, because the entry below would capture every unmatched app in `live.rs`. Install `org.example.Tool.desktop` with `Exec=<std::env::current_exe()>`. Map a toplevel `org.example.Stray`. The tile's accessible label is the entry's name, and Keep saves `org.example.Tool`.

4. **Keep an app that has no entry** (~2 h)
   - `crates/libs/xdg`: `write_launcher(data_home, app_id, proc_root, pid) -> io::Result<PathBuf>` writes `<data_home>/applications/<app_id>.desktop`:
     - `Exec` is the quoted canonical exe; for a `LAUNCHERS` interpreter it is the interpreter plus the script made absolute against `<pid>/cwd`. No other arguments are kept.
     - `TryExec` is the exe, so a deleted dev build hides itself like an uninstalled app.
     - `Name` comes from `app_name`, `StartupWMClass` is the app_id, `Icon` is the app_id when `find_icon` finds it, and `X-Bande-Kept=true`.
     - It never overwrites an existing file.
   - `dock_client.rs`: when Keep is picked for an app with no entry, write the launcher, add the entry to `entries` right away (the watcher picks it up too), then keep it. Disable Keep when there is no entry and no pid, so it never looks done when it isn't.
   - Tests:
     - In `xdg.rs`, the written file parses through `DesktopEntry::parse`. `command()` round-trips an exe path that contains a space, `find_app(app_id)` finds it, and an existing file is left alone.
     - In `keep.rs`, keep `org.example.Stray` with no matching entry, close its window, and check the tile is still there with its label. Its file is in the test `XDG_DATA_HOME` with `Exec` set to the test binary.

5. **The UI says Taskbar** (~1 h)
   - `bande-bar.ftl`: `menu-keep = Keep in Taskbar` and `taskbar = Taskbar` (rename the id at `dock.rs:2516`).
   - `bande-settings-panes.ftl`: `bar = Taskbar`, `bar-summary`, `bar-auto-hide = Automatically hide and show the taskbar`; keep "dock" and "bar" as search keywords.
   - `org.bande.Settings.desktop` Comment; `bande-keyboard.ftl` `focus-dock` text.
   - Tests: the existing message-id test (`crates/shell/bar/src/lib.rs:26`). Regenerate any Settings or bar golden that shows the changed text and look at the diff.

6. **The code says taskbar** (~2 h, mechanical; schedule it when no other branch touches `crates/shell/bar` or compositor `dock.rs`)
   - `git mv`:
     - `crates/libs/dock-model` to `crates/libs/taskbar-model` (crate `bande-taskbar-model`; update `Cargo.toml:86` and `Cargo.lock`);
     - `bar/src/dock.rs` to `taskbar.rs` and `dock_client.rs` to `taskbar_client.rs` (`Dock*` to `Taskbar*`, the `include_str!` list in `lib.rs`);
     - `bar/tests/dock.rs` to `taskbar.rs`, with the goldens `taskbar-{left,bottom}-light.png`;
     - compositor `src/dock.rs` to `apps.rs` and `tests/headless/dock.rs` to `taskbar.rs`.
   - Layer namespace `"dock"` becomes `"taskbar"` in the compositor and the bar. Compositor and shell ship in the same release, so nothing needs migrating.
   - `Action::FocusDock` becomes `FocusTaskbar` and `focus-dock` becomes `focus-taskbar`. Update the protocol XML descriptions and the log test string.
   - Test: the full gate (fmt, workspace clippy `-D warnings`, workspace tests on the hermetic bus). Goldens must be byte-identical after the move. Check that `git grep -i '\bdock'` only hits history, the macOS survey text and `Dockerfile`.

7. **The docs say taskbar** (~1 h)
   - `git mv docs/design/features/dock.md taskbar.md` and fix its links. Update the current-state docs (`bar.md`, `architecture/overview.md`, `behaviour.md`, `compositor.md`, `performance.md`, `quality/userland.md`, `roadmap.md`, `parity.md`) and the `ci/review/scenarios` shot names.
   - Add one line to `taskbar.md` on how Keep resolves an app: entry, then trusted scope, then exe, then a written launcher.
   - Leave decision records and research notes as they are.

## Risks

- **Wrong app.** Trusting an inherited scope would pin Terminal instead of the app started in it. Rule (a) in milestone 2 only accepts a scope whose pid is the window's own, and a test pins this down.
- **Written launchers.** They cover dev builds in `target/` or `/tmp`. `TryExec` hides the tile once the binary is gone, the same way an uninstalled app hides. Arguments are dropped, so `code ~/proj` keeps plain VS Code. That is intended: the taskbar keeps apps, not documents.
- **Flatpak.** `/proc/<pid>/exe` points inside the sandbox, so only the scope works there. That covers Flatpak, since Flatpak always scopes its apps.
- **Pid reuse or wrappers.** The pid is the Wayland client's, and it is read when Keep is picked, so a wrapper script's child is what gets captured. That is usually right.
- **Rename churn.** Milestone 6 conflicts with any open branch touching the bar, including the `taskbar-icons.md` work. Do it last and alone.
- **Today's config.** It already holds `com.microsoft.VSCode`, `dev.zed.Zed` and `com.anthropic.Claude`. These appear correctly after the next login even before any fix.

## Decisions for the owner

Decided overnight 2026-11-01: "Taskbar" for the whole combined bar, Settings ▸ Taskbar (plan recommendation; owner may revisit).
Decided overnight 2026-11-01: written launchers show in the launcher (plan recommendation; owner may revisit).
Decided overnight 2026-11-01: keep the `bar { … }` config key (plan recommendation; owner may revisit).

1. **Name scope.** Recommended: "Taskbar" for the whole combined bar (Settings ▸ Taskbar). The alternative is "Taskbar" only for the apps area, keeping "Bar" as the whole.
2. **Written launchers show in the launcher.** Keeping an app with no desktop file adds a launcher for it in `~/.local/share/applications`, so it also appears in the launcher and search. Recommended: yes, as macOS treats anything in the Dock as an app. The alternative is to mark it hidden from the launcher, which needs a separate exception in `applications()`.
3. **Config key.** Recommended: keep `bar { … }` in `config.kdl`. Renaming it to `taskbar { … }` means reading both keys and rewriting on the next save.

## Sources

1. GNOME Shell, `get_app_for_window` in `src/shell-window-tracker.c`: https://gitlab.gnome.org/GNOME/gnome-shell/-/blob/main/src/shell-window-tracker.c
2. KDE Plasma, `serviceFromMetadata` and `servicesFromPid` in `libtaskmanager/tasktools.cpp`: https://invent.kde.org/plasma/plasma-workspace/-/blob/master/libtaskmanager/tasktools.cpp
3. Windows taskbar pinning stores a shortcut to the exe under `User Pinned\TaskBar` (from knowledge, not fetched).
