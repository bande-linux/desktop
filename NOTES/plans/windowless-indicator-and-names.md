# Windowless indicator and taskbar names

## (a) A windowless app keeps the active bar

**Seen**: close a Bande app's last window with the close button. The app keeps running and
stays active with its App menu (right, as on macOS), but its taskbar icon keeps the accent
active bar instead of the running dot.

**Owner decision (taken)**: keep the app active with its menus and key equivalents, as macOS
does; only the indicator changes: the active bar means "active and has a window", a windowless
app shows the running dot even while active. (macOS's Dock has no active marker at all.)

**Cause**: the compositor keeps the app active on purpose (c2ee81bd): `refocus_after_losing`
(`compositor/src/state/focus.rs:97-117`) sets `windowless_app` and `announce_active_app`
(`shell.rs:249-268`) sends `active_app(<that app>)`; that stays. The taskbar draws the active
bar for any `active == app_id` (`bar/src/taskbar.rs:1958`, set from `ActiveApp` in
`bar/src/taskbar_client.rs:958-962` via `Taskbar::activated`, `taskbar.rs:517`) without asking
whether the app still has a window.

**Fix** (shell only; compositor and menus unchanged):
- Toolkit `crates/toolkit/platform/src/shell.rs`: `Window::is_windowless(app_id)` reading the
  `windowless` list it already keeps from `bande_shell_v1.windowless_app` (it raises
  `AppsChanged` when the list changes).
- `taskbar_client.rs`: keep the raw active app id; on `ActiveApp` and `AppsChanged` hand the
  taskbar `active.filter(|app| !window.is_windowless(app))` (mapped through `identity`).
  `Taskbar::activated` keeps clearing attention and urgency on activation; split out a
  `set_active(Option<&str>)` for the indicator so a windowless active app is drawn by the
  running-dot branch of `paint_rail_indicators`.
- Also covers activating a windowless app from the shell: menus show, the icon keeps its dot
  until a window maps (the reopened window then brings the bar back through `AppsChanged`).
- Docs: `behaviour.md` Taskbar (active bar only while the active app has a window); glossary.

**Risk**: low; one shell process, no protocol change. A non-Bande app that hides its last
window without quitting is not in the windowless list but also not in `running_apps`, so it
shows nothing, as today.

**Verify** (owner, on the mini): open Terminal, close its window with x: the icon shows the
running dot, the App menu still shows Terminal's menus, Ctrl+N opens a window and the active
bar returns. No new test. **Size**: ~25 lines in `taskbar_client.rs`, `taskbar.rs` and the
toolkit, two doc lines.

## (b) Names for the taskbar's parts

Glossary written to `docs/architecture/glossary.md` with the recommended names. Owner decides
the final names; recommended: Bande menu (logo), App menu (header + cascade), Search (the
Super+Space card; owner said Ctrl+Space, which is not bound), Menu Search (Shift+Super+/),
Dock (the app icons area), active bar / running dot, status items, Control Center button,
Clock. Alternatives: "Launcher" instead of Search, "app strip" instead of Dock.

Rename list for later (one branch after the owner decides; code renames are optional):
- `behaviour.md` Taskbar/Apps strip: "Bande mark" -> Bande menu, "active app's header" -> App
  menu, field "Search menus" -> "Type to search" (code wins), "accent tick"/"3 pt square" ->
  active bar/running dot, "status notifier items" -> status items, "Apps strip" -> Dock,
  "Launcher" -> Search.
- `bar` l10n: `launcher = Open the launcher` -> "Search"; `bar` and `taskbar` are both
  "Taskbar" (drop one); `status-area = Status` -> "Status items".
- `bar/src/menu_bar.rs` `Slot::System` -> `BandeMenu`, `Slot::Launcher` -> `Search`,
  `Slot::App` -> `AppMenu` (today `AppMenu(usize)` is a menu bar title: -> `AppMenuTitle`),
  `Slot::Tray`/`TrayMore` -> `StatusItem`/`MoreStatusItems`.
- `bar/src/rail.rs` `RailView` -> `TaskbarView`, `paint_header` -> `paint_app_menu`;
  `taskbar.rs` `Rail` -> `TaskbarLayout`, `RAIL_TICK` -> `ACTIVE_BAR`, `RAIL_RUNNING` ->
  `RUNNING_DOT`, `paint_rail_indicators` -> `paint_indicators`.
- Not renamed: crate `bande-launcher`, namespace `bar`, `workspace` in code (UI says Desktop).
