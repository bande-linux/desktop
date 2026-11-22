# Apps

Every Bande app is built on `bande-app` (`crates/libs/app`) and lives in `crates/apps/<name>`,
packaged as `bande-<name>`: Files, Settings, Dictionary (Look Up), Terminal, Software, Package
Installer, System Monitor, Mail, Calendar, Contacts, Velox, Calculator, Player, Viewer, Clock, Text Editor,
Disks and Archives. The file chooser portal and the keyring prompt are the two programs there that
are not. `crates/apps/template` is the starting point of a new app (`ci/new-app.sh <dir> <Id>
"<Name>"`), a real app that is checked and tested with the others and never released. Code that
several apps share is a library in `crates/libs` (`bande-discovery`: DNS SRV and TXT service
discovery for account setup; `bande-terminal-view`: the terminal widget; `bande-vm`: QEMU virtual
machines in systemd user units; `bande-keyring`: passwords in the Secret Service; `bande-dav`:
CalDAV and CardDAV with offline sync; `bande-content-lines`: iCalendar and vCard content lines;
`bande-address-book`: vCard, the offline address books and their CardDAV sync).
`ci/check-apps.sh` checks that every app is built and packaged this way. A crate's own
`README.md` describes an app where it needs more than its metainfo (Mail, Calendar, Contacts, Clock,
Velox).

## The boundary

- **Toolkit** (`bande-toolkit`): everything that draws, lays out or handles input inside a
  window; windows and their roles; the theme and `Appearance`; alerts and sheets.
- **`bande-app`**: the process and its place in the desktop: `main`, `--version`, logging, the
  window list and focus, close and quit, the app's menus (App, Edit and Window menus, About,
  Settings…) and the menu button (⋮) that shows them in every titled window, the standard keys, appearance for every window, the Settings window, single
  instance and activation, the one session bus connection, state file paths. It draws nothing.
- **The app**: its views (`View`), its own menus and commands, workers and sources, its
  arguments (`Program`).

## Rules every app gets

| What | Rule |
| --- | --- |
| Process | one process for all of an app's windows, Settings included; a second launch hands its arguments, working folder and activation token to the first and exits |
| Menus | Shown from the menu button (⋮) left of each titled window's controls: App menu (About with the app's own version, Settings… if the app has settings, Hide, Hide Other Apps, Show All Apps, Quit), the app's menus, then Window (Minimize, Maximize, Close Window, the app's window items) |
| Keys | 1. Close Window, Quit and Settings… (`Ctrl+W`, `Ctrl+Q`, `Ctrl+,`; with `Shift` for Terminal) before the view. 2. The focused view. 3. A key the view did not take runs the enabled menu entry with that shortcut, so the menus are the one shortcut table |
| Edit menu | `edit_submenu` items are `key:<shortcut>` actions: the focused view gets that key press, so Copy works in any text field |
| Close | Close Window closes the active tab first where the app has tabs (`Program::close_tab`), else asks `should_close`; the close button closes the window; closing the last window leaves the app running until Quit; Quit asks every window and stops at the first refusal |
| Without windows | the app stays in the Taskbar and, when it was active, the active app: its shortcuts still reach it and its menus are in the Taskbar icon's menu and Search, as full as with a window: what needs a window (Minimize, Maximize, Close Window, the Edit menu's keys, the app's own window items) is disabled, and the app's items (New Message, a View item, Window ▸ its main window) work and open a window when they need one; Hide makes the next app active; a Taskbar click or a second launch calls `Program::reopen`, while the switcher only makes it the active app; `--no-window` starts it running and active with no window (the desktop starts Files this way), and does nothing when it already runs; `Program::window_closed` hands back a closed window's view so the app can keep it; program sources and deadlines keep running, window resources are released |
| Resume | every window with a saved state comes back on the next plain launch, with its display, workspace, frame and state from the compositor ([behaviour](behaviour.md)). `Program::save_window` returns a `Saved` (any serialisable value, by default empty for `Kind::Main`), `Program::restore_window` opens the window (by default `reopen`); the first window opened inside it takes the saved place. `start` only opens what its arguments ask for: `bande-app` then restores, or calls `reopen` if there was nothing. Saved to `$XDG_STATE_HOME/bande/resume/<app id>` from a worker thread within a second of a change (`Cx::state_changed` for a change no event shows). Closing a window forgets it; Quit forgets them all and removes the session; log out (`session_ending`, read with `Cx::session_ending`), a lost display and quitting an app whose code an update replaced keep them, so "Quit and reopen" brings its windows back. The Settings… window comes back on its own. If the app dies within seconds of reopening, the next launch asks before trying again |
| In the background | an app whose `Program::runs_in_background` says so (Calendar while its "Show alerts when Calendar is closed" setting is on) can start with `--background` (from its XDG autostart entry) and Quit leaves it running there too, so its alerts keep working: it has no window and is no running app (not in the Taskbar, Force Quit, the switcher or the apps reopened at login) and its sources and deadlines keep running. `Cx::launched_in_background` tells `Program::start` to open no window. A later launch reaches it through single instance and `Program::reopen` opens its window; from its first window it is an ordinary running app until Quit. `--background` exits at once when the app does not run in the background or is already running |
| Memory without windows | a closed window takes its renderer with it: the GPU device once no window uses it, glyph and image atlases, the shown scene and the window's text caches; with no window left `bande-app` hands freed heap back (`malloc_trim`). A program keeps a closed window's view (`Program::window_closed`) and shows it again on Reopen: a kept view costs 1–2 MB beyond its data (Terminal, which keeps none, ends 3.4 MB of private memory after its window closes; Mail 5.4, Calendar 4.7 against 2.7 started in the background) and brings back unsent text, selection and scroll in 25–80 ms, so views are kept, not rebuilt, unless one holds large caches of its own, which it drops in `window_closed`. Mesa's Vulkan drivers and LLVM stay loaded once a window has drawn on the GPU: about 9 MB more private memory and 55–60 MB of shared library pages until the app exits. `bande-background` measures it |
| Appearance | every view gets each change; a view is built with the window's real appearance |
| Other apps | Show in Files is `bande_app::show_in_files`, `org.freedesktop.FileManager1.ShowItems` off the UI thread. An action that opens another app (Show in Files, Open in Terminal, Software Update) is left out while that app is not installed, so no menu offers what cannot work: a menu built when it opens asks `bande_xdg::is_installed`; menus and views that stay up ask `Cx::installed`, which follows the XDG `applications` folders with inotify from the first call, rebuilds the menus and calls `Program::installed_changed` when they change |
| Dropped files | `bande_app::FileDrop` takes files dragged onto a window as a copy and hands them to the view, which opens, attaches or imports them; a drag of text still goes to the widget under the pointer |
| Tabs | a tab dragged out of a window's `TabStrip` floats in a new window that follows the pointer and can be dropped into another window's strip: the view implements `TabHost` (release, adopt, slide, insertion index, grab point), its drag events go through `TabDropTarget`, `move_tab` reorders a window's list and keeps the active tab active, and the program keeps one `TabCarry` that moves the tab between windows and puts it back when the drag is cancelled |

## Contracts with the shell

The shell and an app meet at: the dbusmenu export at `/org/bande/MenuBar` with
`x-bande-shell-action` and `x-bande-version`; `bande_app_v1.keep_running` (sent with the first
window and undone by destroying the object for the background); the app id as the bus name;
`org.freedesktop.Application` (`Activate`, `Open` with `activation-token` and
`x-bande-directory` in the platform data) at the path the specification derives from the app id;
`<package> --version`; the state (`$XDG_STATE_HOME/bande/`) and config (`$XDG_CONFIG_HOME/bande/`)
paths.

## Services and popovers
A program whose `Program::waits_for_activation` says so (Dictionary, for Look Up) is started by D-Bus with no
window, waits for its first request and exits once its last window has closed, unless a window it
opened asked to keep running. `Kind::Popover`
opens a window at the pointer that takes the keyboard, has no title bar or menu button and is not
resumed, kept running or listed as a window. See [lookup.md](lookup.md).

## Printing
An app with a document puts `bande_app_menu::print_item(enabled)` in its File menu and handles the action `bande_app_menu::PRINT` with `bande_app::print(title, id, Document)`: `Document::Pages` lays out pages for a `PageSetup` on a worker thread (with `bande_app::flow` for flowing text, or `Page::draw` with scene items), `Document::File` prints an existing PDF. The print dialog is `bande-file-chooser --print`, one program for every app, attached modally to the asking window through xdg-foreign: a preview of the actual PDF drawn in the confined `--pdf-worker` child (so the preview is what prints), printer, copies, pages, paper, orientation, two-sided, colour and Save as PDF. With `Document::Pages` the dialog asks the app for a new layout when paper or orientation changes, so text reflows; other apps come through the portal's `org.freedesktop.impl.portal.Print`. Pages become PDF through `bande-pdf` (krilla): vector, subset fonts, selectable text.

## App settings
`Info.settings` gives an app one Settings… window in its own process: fixed size, one scrolling `bande_toolkit::ui::Form` sheet (panes as toolbar tabs, the window titled by the selected one). Each change is written at once, atomically, to `$XDG_CONFIG_HOME/bande/<app>.conf` (`key = value`); unknown or broken values fall back to defaults. Every window watches the file (`bande_app_settings::SettingsWatch`) and applies changes live; the last write wins.
