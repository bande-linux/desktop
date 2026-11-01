# Apps

Every Bande app is built on `bande-app` (`crates/libs/app`): Files and Settings in this repo, and
every app in [`bande-linux/apps`](https://github.com/bande-linux/apps) (Terminal, Activity
Monitor, Software, the Package Installer, Mail, Calendar and the app template), which is one
Cargo workspace pinning the desktop crates at one revision. `ci/check-apps.sh` checks that every
app here is built this way.

## The boundary

- **Toolkit** (`bande-toolkit`): everything that draws, lays out or handles input inside a
  window; windows and their roles; the theme and `Appearance`; alerts and sheets.
- **`bande-app`**: the process and its place in the desktop: `main`, `--version`, logging, the
  window list and focus, close and quit, the menu bar (App, Edit and Window menus, About,
  Settings…), the standard keys, appearance for every window, the Settings window, single
  instance and activation, the one session bus connection, state file paths. It draws nothing.
- **The app**: its views (`View`), its own menus and commands, workers and sources, its
  arguments (`Program`).

## Rules every app gets

| What | Rule |
| --- | --- |
| Process | one process for all of an app's windows, Settings included; a second launch hands its arguments, working folder and activation token to the first and exits |
| Menu bar | App menu (About with the app's own version, Settings… if the app has settings, Hide, Hide Others, Show All, Quit), the app's menus, then Window (Minimize, Zoom, Close Window, the app's window items) |
| Keys | 1. Close Window, Quit and Settings… (`Ctrl+W`, `Ctrl+Q`, `Ctrl+,`; with `Shift` for Terminal) before the view. 2. The focused view. 3. A key the view did not take runs the enabled menu entry with that shortcut, so the menus are the one shortcut table |
| Edit menu | `edit_submenu` items are `key:<shortcut>` actions: the focused view gets that key press, so Copy works in any text field |
| Close | Close Window closes the active tab first where the app has tabs (`Program::close_tab`), else asks `should_close`; the close button closes the window; closing the last window leaves the app running, as on macOS, until Quit; Quit asks every window and stops at the first refusal |
| Without windows | the app stays in the Taskbar and, when it was active, in the menu bar, whose key equivalents still reach it; a Taskbar click, the switcher or a second launch calls `Program::reopen`; `Program::window_closed` hands back a closed window's view so the app can keep it; program sources and deadlines keep running, window resources are released |
| Appearance | every view gets each change; a view is built with the window's real appearance |

## Runtime contracts

Apps built at an older pin run against a newer shell, so these only ever gain properties:
the dbusmenu export at `/org/bande/MenuBar` with `x-bande-shell-action` and `x-bande-version`;
`bande_app_v1.keep_running` (version 2; an older compositor makes the app quit with its last window);
the app id as the bus name; `org.freedesktop.Application` (`Activate`, `Open` with
`activation-token` and `x-bande-directory` in the platform data) at the path the specification
derives from the app id; `<package> --version`; the state (`$XDG_STATE_HOME/bande/`) and config
(`$XDG_CONFIG_HOME/bande/`) paths.
