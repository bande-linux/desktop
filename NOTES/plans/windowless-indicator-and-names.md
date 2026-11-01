# Windowless indicator and taskbar names

## (a) A windowless app stays active after its last window closes

**Seen**: close a Bande app's last window with the close button. The app keeps running (right),
but its taskbar icon keeps the accent active bar and the App menu (the header under the logo
and its "Type to search" cascade) keeps that app's menus.

**Cause**: deliberate, from c2ee81bd ("Keep an app active in the menu bar after it closes its
last window"), which followed macOS. `State::refocus_after_losing`
(`compositor/src/state/focus.rs:97-117`) looks only for another window of the *same* app
(`frontmost(Some(owner))`); with none it focuses nothing, sets `windowless_app` to the closing
app and calls `announce_active_app`, which falls back to `windowless_app`
(`shell.rs:249-268`) and sends `active_app(<that app>)`. The shell only mirrors that event:
the taskbar draws `RAIL_TICK` for `active == app_id` (`bar/src/taskbar.rs:1950-1983`), and
`MenuBarClient` loads that app's menus for the App menu (`bar/src/menu_bar_client.rs:557-569`).
No shell change is needed.

**macOS** keeps the app frontmost with its menu bar (Cmd+N/Cmd+Q still reach it); the owner
wants it to stop being active instead.

**Fix** (compositor only, `refocus_after_losing`): pick
`frontmost(owner).or_else(|| frontmost(None))` and `focus` it; drop the `windowless` branch.
- Focus goes to the next app's frontmost window on the active workspaces of any display (the
  rule `release_exclusive_layer` uses); `sync_activated` announces it, so the active bar and the
  App menu move to that app.
- No window left on the active workspaces: focus is cleared and `active_app("")` is sent; the
  App menu slot disappears (`rail.rs:258`), no icon has the active bar, the closed app keeps its
  running dot. Other workspaces are not switched to.
- Unchanged: a taskbar click, the switcher or a second launch reopens the app; the shell's
  `activate_app` of a windowless app still activates it without a window (the only path left
  into `windowless_app`, so `activate_next_app_if_the_windowless_one_quit` stays).
- Docs: `apps.md` "Without windows" row: the app stops being active and focus goes to the next
  app's frontmost window, else none; note the macOS difference in `parity.md`.

**Risk**: Ctrl+Q right after closing the last window now quits the *next* app (on macOS it
quits the closed one), and Ctrl+N opens a window of the next app. Accepted by the owner's wish.

**Verify** (owner, on the mini): Terminal and Files open; close Terminal's window with x:
Terminal gets the running dot, Files gets the active bar and its App menu; with no other window
left the App menu is gone and no icon is active; clicking Terminal reopens a window and it is
active again. No new test (`libs/app/tests/app.rs` covers close-last-window then reopen).
**Size**: ~10 lines in `focus.rs`, two doc lines.

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
