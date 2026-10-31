# App settings windows (Files first)

Gap (owner): "Files has no 'settings' (and other inhouse apps who needs it), for example i want
to show file extensions".

## Summary

Take Terminal's settings window, which already works, and turn it into a shared pattern: a
small per-app file under `~/.config/unbound/`, saved as you change it and followed live by every
window, plus one shared form widget and one shared "Settings…" menu entry. Files gets a settings
window first. Terminal moves onto the shared code. No other app needs one yet.
Files already shows every extension. Long names are cut at the end in list and column rows, and
that is what hides the extension. Truncating in the middle fixes it.

## What the owner sees

- Files ▸ Settings… (`Super+,` today; `Ctrl+,` if `NOTES/plans/super-ctrl.md` is adopted) opens a
  "Files Settings" window that looks like Terminal Settings: one scrolling sheet with headings
  and a label and control on each row. A change applies at once, in every Files window, and
  is still there the next time Files starts.
  - **General**: New windows show (Home, Desktop, Documents, Downloads, Recents, Last Folder);
    Default view (Icons, List, Columns); Show hidden files; Keep folders on top.
  - **Sidebar**: a toggle for each built-in place (Recents, Home, Desktop, Documents, Downloads,
    Music, Pictures, Videos, Trash, Internal, external disks).
  - **Trash**: Warn before emptying the Trash; Remove items from the Trash after 30 days.
  - **Search**: Search in (Current Folder, Home).
- `Super+Shift+.` flips "Show hidden files" for every window, as in the Finder, and the
  setting is remembered.
- A long name such as `quarterly-report-final-version-2.pdf` shows as `quarterly-rep…ion-2.pdf`
  in list and column rows, so the extension stays visible. Icon view already works this way.
- Terminal Settings looks and works as it does today.
- Activity Monitor, Screenshot, System Settings and the file chooser get no settings window.

## Findings

**Terminal has the only app settings window, and it is a good base.**
- Storage: `key = value` lines in `$XDG_CONFIG_HOME/unbound/terminal.conf`. Unknown or broken
  values fall back to the defaults, and the file is written atomically (stage, then rename)
  (`crates/apps/terminal/src/settings.rs:11,84-151`).
- Window: a separate process, `unbound-term --settings`, with the same app id
  (`terminal/src/main.rs:49`, `preferences.rs:76-89`). Super+, starts it through `spawn_self`
  (`app.rs:880`, `app.rs:1893`). The window saves after every event if anything changed
  (`preferences.rs:30-43`).
- Live apply: each Terminal process watches the folder with `unbound_watch::FolderWatch`
  (`Changes::Saved`), keeps a wake socket among its sources and reloads the file when woken
  (`app.rs:117-147, 1109-1124, 1723, 1746-1749`).
- Layout: a private `Form` widget with three kinds of line: Heading, Row (label on the left,
  control on the right, the label wrapping at 55% of the width) and Preview. It is inside a
  `ScrollView` and follows the text size (`settings_view.rs:34-229, 325-333`). The controls are
  toolkit widgets: `PopUpButton`, `Stepper`, `SegmentedControl`, `Toggle`.
- Tests: parsing keeps defaults and round-trips (`tests/term.rs:877`). Goldens at 1× and 1.5×
  (`tests/term.rs:943`, `tests/golden/term-settings*.png`).
- Menu: Terminal adds its own `Command::Settings` row to `app_submenu`, shortcut "Super+," as a
  string literal (`menu.rs:73,255-258`). The key itself is matched in `keys.rs:288`.

**The app menu has no standard Settings entry.** `app_submenu(app, items)` builds About, then
the items, then Hide / Hide Others / Show All / Quit (`crates/libs/app-menu/src/lib.rs:85-114`).
Files, Monitor and System Settings pass no items (`files/src/menu_bar.rs:27`,
`monitor/src/menu.rs:143`, `settings/src/main.rs:83`). The bar design expects "Settings…" in
the app row (`docs/design/features/bar.md:114`). `Super+,` is the documented shortcut
(`docs/architecture/overview.md:151`).

**org.unbound.Config is for system settings and costs a lot per key.** Every setting is a
field on `Config`, a KDL parse and edit, and a D-Bus property with a setter
(`crates/services/config/src/lib.rs:645-664`, `service.rs:432-620`). It has no generic per-app
store.

**Files keeps its UI state in loose files and applies no settings.**
- State files under `$XDG_STATE_HOME/unbound/`: `files-views` (view per folder),
  `files-sidebar` (order, plus hidden places as `-token` lines), `files-columns` and
  `files-status-bar` (`files/src/views.rs:82-160`, `sidebar.rs:65,145-210`).
- Every window is its own process. New Window runs `unbound-files <path>`
  (`main.rs:178, 372-387`), and one-process windows are planned in F24
  (`docs/experience/files.md:547`). Live apply therefore has to reach other processes. A file
  watch does that, and it keeps working after F24.
- Hidden files: a per-window flag that starts off and is never saved
  (`browser.rs:274, 1103`; `view.rs:1503, 1615`).
- Default view: Columns for any folder without a remembered view (`views.rs:12-18`,
  `view.rs:344`).
- Folders always come first. This is hard-coded in `unbound_fs::compare`
  (`libs/fs/src/listing.rs:91`).
- Start location: Home unless a path is given (`main.rs:390-401`).
- Empty Trash always asks (`controls.rs:1030-1036`, `context.rs:227`).
- Search scope: the current folder, or Home when the search starts there (`search.rs:145-161, 200`).
- Sidebar: places can be removed and the whole sidebar restored ("Restore Default Sidebar"),
  but one hidden place cannot be shown again on its own (`context.rs:290-309`). A window
  re-reads the sidebar only after its own edits (`main.rs:102, 184`).
- **Extensions are never hidden.** Names are drawn from `entry.name` as they are. List and
  column rows clip the name at the row edge (`paint.rs:542-547`), so a long name loses its
  extension. The icon grid already truncates in the middle (`grid.rs:134, 284-311`). The Files
  polish checklist asks for exactly that: "keep the extension visible"
  (`docs/experience/files.md:591`).
- Already planned: "Remove from Trash after 30 days" (later, one checkbox; `docs/design/features/files.md:191`,
  `docs/experience/files.md:153`). Folders first (`features/files.md:222`). Columns as the
  default view (`features/files.md:229`). A View Options panel is listed under "never"
  (`features/files.md:202`).
  `TrashItem.deleted` already holds the deletion date (`libs/fs/src/trash.rs:27-32`).
- `wip/files-depth-3` (unmerged, `92784c6e`) touches `browser.rs` (`read_folder`, which is
  where `show_hidden` is used), `paint.rs` (empty states), `controls.rs` (the sheet list),
  `search.rs` and `sidebar.rs` (Internal disk name). These are the same files, so there will be
  textual conflicts but no conflict in design.

**Other apps do not need a window.**
- Activity Monitor: an update-frequency setting is a deliberate "never". The answer is a cheap
  2 s update (`docs/experience/monitor.md:121,159`; `monitor/src/main.rs:25`). "Which processes"
  is already in View (All, Mine, Apps) and remembered (`monitor/src/menu.rs:128-131`,
  `model.rs:868-887`).
- Screenshot: save location, timer and pointer are already in its Options menu and remembered
  (`shell/screenshot/src/options.rs`).
- System Settings is itself the settings app. The file chooser lists folders and mirrors
  nothing from Files (`file-chooser/src/model.rs:168`).

## Options

**Storage**
- **A. One file per app**: `~/.config/unbound/<app>.conf` with `key = value` lines, followed by
  inotify (the Terminal model). It needs no service, works when the bus or the config service
  is down, and is proven in daily use. The cost is a second format next to `config.kdl`.
- **B. Typed sections in `config.kdl` through org.unbound.Config.** Everything lives in one
  file. But each Files setting needs a `Config` field, KDL parse and edit code, a D-Bus property
  and a client proxy, and Files would depend on the service at start-up. That is about four
  times the code per setting.
- **C. A generic `app "org.unbound.Files" { … }` block in `config.kdl`**, with string-typed
  `AppSettings`/`SetAppSetting` methods and a change signal. This is one file without code per
  key, but it is a new service API, still needs the bus, and loses typing at the boundary.

**Settings window**
- **A. A separate process, `<app> --settings`** (as Terminal does today). Every app works the
  same way. It survives F24 unchanged. A second press of Super+, opens a second window.
- **B. A second surface in the app process.** It needs `run_surfaces` in Files (F24) first, and
  each app would do it differently.

**Extensions**
- **A. Always show them and truncate in the middle** in list and column rows. One small change,
  and the extension is always visible.
- **B. Add the Finder's "Show all filename extensions" toggle**, which hides known extensions
  when off. Linux has no per-file "hide extension" flag. Hiding extensions makes `invoice.pdf.sh`
  look like `invoice.pdf`. Rename and batch rename would need a shown-name versus real-name
  split.

## Recommendation

Storage A, window A, extensions A. The shared parts go in one new lib, `crates/libs/app-settings`
(file path, atomic write, `key = value` pairs, the watch, the generic settings window). A
`Form` widget goes into `toolkit/widgets` and a `settings()` entry into `app-menu`, so the
shortcut is defined in one place for the Super→Ctrl change. Terminal moves first, and its
goldens prove that nothing changed. Then Files gets its settings in small, separately useful
steps. `Super+Shift+.` becomes the persisted global toggle, with no per-window flag.
Leave out update frequency, a View Options panel, settings windows for Monitor, Screenshot or
the file chooser, and tabs inside the settings window (one scrolling sheet is enough for about
15 rows).

## Implementation plan

Each milestone is one branch from `main` and leaves it green. Files milestones 5 to 9 land after
`wip/files-depth-3` merges, or rebase onto it.

1. **Keep extensions visible in list and column rows.** This is independent and is the owner's
   actual symptom. Move `middle_truncated` and the fitting loop from `files/src/grid.rs:134,284`
   into a shared single-line helper in `files/src/paint.rs`, and use it for the row names
   (`paint.rs:542`) and the list name column. Keep the result cached as the grid does.
   Tests: a long name truncated to a row width ends with its extension and fits the width; a
   short name is unchanged; a folder name is truncated in the middle too. Update the column and
   list goldens with one long name.
2. **`unbound-app-settings` lib: file and watch.** New `crates/libs/app-settings` with
   `settings_file(name)` (`$XDG_CONFIG_HOME/unbound/<name>.conf`), `read`, `write` (stage, then
   rename), `pairs(text)` and `flag(value)` (moved from `terminal/src/settings.rs:76-118`), and
   `SettingsWatch` (moved from `terminal/src/app.rs:117-147`, filtered to one file name). Move
   Terminal onto it and delete the copies. Add it to the workspace `Cargo.toml`.
   Tests (tempdir, real inotify): a write wakes the watch; a write to a sibling file does not;
   after `write` no staging file is left and `read` returns the text. The existing terminal
   settings test stays green.
3. **`Form` widget in the toolkit.** Move Terminal's `Form` (`settings_view.rs:27-229`) to
   `crates/toolkit/widgets/src/form.rs`, export it through `unbound_toolkit::ui`, with lines
   `Heading(text)`, `Row(label, control)` and `Wide(widget)`. Terminal's preview becomes a
   `Wide` child widget in `terminal/src/settings_view.rs`.
   Tests: `term-settings` and `term-settings-150` goldens are unchanged; a row whose label
   wraps to two lines is taller than one row, and its control stays vertically centred.
4. **Generic settings window and the standard menu entry.** Move `Preferences`
   (`terminal/src/preferences.rs`) into `app-settings` as `SettingsWindow`, generic over a
   small `Stored` trait (`parse`, `to_text`, `PartialEq`). It saves on change and also follows
   the watch, so a change made elsewhere (`Super+Shift+.`, a second settings window) rebuilds
   the form. In `libs/app-menu` add `Standard::Settings` (`app-menu:settings`) and `settings()`
   with the one `SETTINGS_SHORTCUT` constant ("Super+,"). Terminal uses both and drops its own
   copy (`menu.rs:73`). `keys.rs:288` keeps matching the key.
   Tests: changing the file under an open `SettingsWindow` updates its shared value and
   rebuilds the form, and does not write back; the app-menu export test (`tests/export.rs`)
   finds "Settings…" with its shortcut and `app-menu:settings` in the app submenu.
5. **Files settings model and window.** `files/src/settings.rs`: `FilesSettings` with
   `show_hidden`, `default_view`, `folders_first`, `new_window` (Home, Desktop, Documents,
   Downloads, Recents, Last), `warn_empty_trash`, `trash_after_30_days` and `search_home`,
   stored in `files.conf`. Add `--settings` next to `--info` (`main.rs:335,408`) to run
   `SettingsWindow` with the General section. Add `settings()` to `menu_bar.rs:27` and
   `Super+,` to the view's keys (`view.rs`, near 1503), raising a `Request::OpenSettings` that
   `main.rs` spawns with an activation token, as `new_window` does.
   Tests: parsing keeps defaults for broken values and round-trips; `files-settings` goldens in
   light at 1× and 1.5×; `Super+,` produces `Request::OpenSettings`; the l10n message-id test
   covers the new strings.
6. **Files applies General live.** Add a `SettingsWatch` to `Files::sources` (`main.rs:275`).
   On wake, reload and apply: show hidden (replaces the per-window flag in `browser.rs:274,1103`,
   and `Super+Shift+.` now writes the setting), the default view for folders without memory
   (`view.rs:344`), and the new-window location in `start_location` (`main.rs:390`; "Last"
   reads the most recent `files-views` entry). Folders first: add `folders_first` to
   `unbound_fs::Sort`, which is not stored per folder (`libs/fs/src/listing.rs:79-96`); the
   file chooser keeps `true`.
   Tests: `libs/fs/tests/fs.rs`, with folders first off, sorts by name across files and folders;
   `files/tests/files.rs`: a browser opened with show-hidden lists dotfiles; `Super+Shift+.`
   writes `show-hidden = true`; a remembered folder view wins over the default view; each
   `new_window` value resolves to the right location; applying a reloaded setting re-lists the
   open columns.
7. **Trash and Search sections.** "Warn before emptying the Trash" skips the alert in
   `context.rs:227` when off. "Search in" sets the initial scope in `search.rs:200`. Add both
   sections to the window.
   Tests: with the warning off, Empty Trash empties a temporary trash with no sheet; with it
   on, the sheet appears; a search started in a subfolder with "Home" scopes to Home.
8. **Sidebar section.** The toggles write the hidden-place list through the existing
   `layout_text` / `edit_sidebar` path (`sidebar.rs:203`, `main.rs:87-100`). Files windows also
   follow `files-sidebar` with the watch from milestone 2, so a change in one window or in the
   settings window shows in all of them (today `main.rs:184` reloads only after the window's own
   edit).
   Tests: turning Desktop off writes `-desktop` and `rows()` leaves it out; turning it back on
   restores it in its old position; a sidebar file changed by another process is reloaded.
9. **Remove items from the Trash after 30 days** (default off). Add
   `unbound_fs::purge_trash(trash, now, Duration)`, which deletes items whose `deleted` date is
   older than the limit and keeps items without a date. Files runs it on a worker at start and
   once a day while it runs, when the setting is on. Update `docs/experience/files.md` (the gap
   rows and a short "Settings" section) and `docs/architecture/behaviour.md` (the app settings
   pattern).
   Tests: a temporary trash with items 10 and 40 days old and one without a date: only the
   40-day item and its `.trashinfo` are removed; nothing is removed while the setting is off.

## Risks

- Conflicts with `wip/files-depth-3` in `browser.rs`, `paint.rs`, `controls.rs` and
  `search.rs`. Land milestone 1 and milestones 5 to 9 after it merges.
- A watch wake comes from any save in `~/.config/unbound/` (for example `config.kdl`). It is
  filtered by file name, as Terminal does, and a reload that changes nothing is a no-op.
- Two settings processes can be open at once. Both follow the file, so they agree (last write
  wins).
- The 30-day purge runs only while Files runs, and it deletes data permanently. The setting is
  off by default and the code is covered by tempdir tests only. Never test it on the real
  Trash.
- The Super→Ctrl change edits the shortcut in `app-menu` and in each app's key match
  (`terminal/src/keys.rs:288` and the Files view). Milestone 4 keeps the menu text in one
  constant, but each app still matches the key itself.

## Decisions for the owner

1. **Extensions**: always show them and truncate in the middle (recommended), or also add a
   "Show all filename extensions" toggle that hides known extensions when off?
2. **Storage**: one small file per app in `~/.config/unbound/` (recommended), or app sections in
   `config.kdl` through org.unbound.Config?
3. **Show hidden files**: one remembered setting that `Super+Shift+.` flips for every window, as
   in the Finder (recommended), or keep it per window and only set the starting value?
4. **Trash after 30 days**: run it only while Files is running (recommended), or as a small
   session service that also runs when Files is closed?
