# Taskbar names

## Names for the taskbar's parts

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
