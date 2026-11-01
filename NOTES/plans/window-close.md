# Closing the last window: apps keep running until Ctrl+Q

Owner decision (2026-11-01): "x closes the main window but the app is still running unless
ctrl + q is pressed which quits the app", and, clarified the same evening, "i mean that for all
bande-apps". One rule for every Bande app: closing the last window (the close button or
`Ctrl+W`) leaves the app running; only Quit (`Ctrl+Q`, App menu ▸ Quit, the Taskbar's Quit,
Force Quit, logging out) ends it. There is no per-app quit-on-close switch.

Before this, `bande-app` quit as soon as the last window closed, so Mail stopped syncing and
notifying and Activity Monitor, Calendar and the rest had to start again from nothing.

## macOS, per app

AppKit's default is to keep running: `applicationShouldTerminateAfterLastWindowClosed(_:)` is
false unless an app returns true, and the Dock keeps the app's dot. The app stays the active
app with its menu bar after its last window closes, its menu key equivalents keep working, and
clicking its Dock icon (or opening it again from Finder or Launchpad) sends the reopen event, on
which the app opens a new main window. A few single-window utilities opt in to quitting.

| Bande app | Apple counterpart | macOS on closing the last window | Bande now | Reopen opens |
| --- | --- | --- | --- | --- |
| Files | Finder | keeps running (Finder never quits) | keeps running | a new window at the default location (Settings ▸ New windows open) |
| Settings | System Settings | **quits** (System Settings and System Preferences always have) | keeps running (owner's rule) | the same window again, on the pane it showed |
| Terminal | Terminal | keeps running | keeps running | a new window with a new shell |
| Activity Monitor | Activity Monitor | keeps running (Window ▸ Activity Monitor, `⌘1`, brings it back) | keeps running, sampling stops | the main window |
| Mail | Mail | keeps running, keeps fetching and notifying | keeps running, syncing and notifying | the same viewer, with its mailbox and selection |
| Calendar | Calendar | keeps running, alerts still fire | keeps running, alerts still fire | the same view, on the day it showed |
| Software | App Store | single-window; not verified on a current Mac | keeps running, installs continue | the same window with its progress |
| Installer | Installer | **quits** | keeps running (owner's rule) | the last package's window |
| template | (document app) | keeps running | keeps running | a new window |

Where macOS differs (System Settings, Installer, and Calculator-style utilities) the owner chose
the one rule for consistency: nothing is lost by keeping them, `Ctrl+Q` is always there, and
the windowless process is kept small (see "Resources").

## Design

### Compositor (`bande_app_v1` version 2, `bande_shell_v1`)

- `bande_app_v1.keep_running(app_id)` (since 2): the client says it is that app and stays
  running without windows. The compositor keeps the resource in `kept_apps`; a kept client with
  no toplevel is a *windowless app* (`State::windowless_apps`).
- Windowless apps count as running everywhere a running app is listed: `app_owners` (so
  `quit_all_apps` asks them, `apps_running` waits for them, and the apps to reopen at the next
  login include them), `app_clients` (Taskbar Quit and Force Quit, Activity Monitor's quit),
  Force Quit's list and the app switcher.
- Quit requests for a kept client go only to the kept object, so an app with several windows is
  asked once, not once per window binding.
- `bande_shell_v1.windowless_app(app_id, pid, windowless)` tells the shell, so the Taskbar keeps
  the running dot (toolkit `Window::running_apps` merges these with the toplevel list).
- Activating and reopening are two requests, as on macOS. `activate_app` makes an app with no
  window the active app (no keyboard focus, its menu bar shown) and opens nothing; the shell
  uses it to hand the keyboard back after Taskbar or menu bar navigation. `reopen_app` (a
  Taskbar click, a notification click, the app switcher) does the same and also sends
  `bande_app_v1.reopen(token)` with an activation token the compositor creates, as the Dock
  sends the reopen event.
- The menu bar: the compositor already kept a windowless app active after its last window
  closed (`windowless_app`, commits c2ee81bd and c622420d); the bar follows `active_app` and reads
  the app's dbusmenu by bus name, so the menu bar stays. New: while such an app is active and no
  surface has the keyboard, keys that are not the compositor's own shortcuts go to it as
  `bande_app_v1.key(keysym, modifiers)`, as macOS sends key equivalents to the active app's
  menus, so `Ctrl+Q`, `Ctrl+N` and `Ctrl+,` work without a window.

### Toolkit

- `AppConnection` (`toolkit/platform/src/app_connection.rs`): the app's own Wayland connection,
  separate from its windows. It binds `bande_app_v1` at version 2 and asks to keep running; on an
  older compositor (or another one) `keeps_running()` is false and the app quits on its last
  window as before. Windows open on its connection; it turns `reopen`, `key` and
  `quit_requested` into `AppRequest`s.
- A closed window now releases its per-window globals (`xdg_wm_base`, seat, keyboard, pointer,
  data and primary-selection devices and any clipboard source it still offered, text input,
  activation, cursor shape, `bande_app_v1`/`bande_shell_v1`). Before, these lived until the
  process exited, which was harmless when the last window ended the app; in a long-running app
  their events (keymaps with fds, selection offers, pings) would pile up in a dead queue, an old
  `xdg_wm_base` would leave a ping unanswered and mark the app as not responding, and a copied
  selection would hang pastes instead of passing to the clipboard daemon.

### `bande-app`

- `Program::reopen` (required): open the main window. Called for the compositor's reopen and for
  a second launch with no arguments while the app has no window (the launcher, `Activate` on
  `org.freedesktop.Application`).
- `Program::window_closed(window, view)` hands a closed window's view back, so an app can park
  its main view (Mail, Calendar, Settings, Software, Installer) and open it again on reopen.
- `Program::command` takes `Option<WindowId>`: menu items and key equivalents arrive with no
  window while the app is windowless.
- The runner alternates `run_surfaces` while there are windows and a poll on the app
  connection, the menu, the single-instance inbox and the program's sources while there are
  none. Program sources and deadlines keep running, view sources stop with their windows.
- Menus while windowless: Minimize, Zoom and Close Window are disabled
  (`Entry::without_window`); the App menu (About, Settings…, Quit) and the app's own menus stay.
- After the last window closes the runner calls `malloc_trim(0)`.
- Quit: `Ctrl+Q` with or without a window calls `should_quit` (Terminal's running-programs
  question, Mail keeping drafts) and ends the process.

### Window menu, Force Quit, logout, restore

- The app's Window menu: window items disabled while windowless.
- Force Quit lists windowless apps (macOS lists every running app); a kill ends them.
- Logout: `quit_all_apps` asks windowless apps too and waits for them.
- Restore at login: windowless apps are remembered; at the next login they start again and open
  their main window, as a launch does.

## Resources

Measured headless (in-process compositor, private session bus, private `HOME`, lavapipe,
debug builds), `smaps_rollup` 8 s after start and 8 s after `Ctrl+W` closed the last window:

| App | With window: RSS / anon MiB | Windowless: RSS / anon MiB | Kept while windowless |
| --- | ---: | ---: | --- |
| Files | 95.6 / 14.5 | 89.7 / 10.4 | nothing |
| Settings | 104.7 / 19.8 | 100.7 / 15.9 | the main view, fed by its services |
| Mail | 98.2 / 15.6 | 93.9 / 11.5 | the viewer and its sync engine |
| Calendar | 96.4 / 15.8 | 91.6 / 11.1 | the view, its library and alerts |
| Activity Monitor | 93.4 / 15.0 | 86.9 / 10.9 | nothing; sampling stops |
| Terminal | 86.5 / 11.5 | 84.1 / 10.2 | nothing; each window's shells end with it |
| Software | 95.3 / 19.8 | 92.0 / 16.5 | the view and its jobs |

Every app used 0 CPU ticks over 8 s without a window (no busy loop on an undrained source).
Threads: Files 12 → 8; the others keep their worker threads.

Most of the remaining RSS is file-backed and shared: lavapipe's `libLLVM` (44 MiB, a test
artefact) and the debug binary's text. GPU buffers (swapchain images, glyph atlases) go with
the window; on the mini's GPU they are device memory, which the lavapipe run cannot show.

## Milestones

1. Compositor: `keep_running`, `reopen`, `key`, `windowless_app`; windowless apps in quit,
   logout, restore, Force Quit and the switcher. Tests: `taskbar.rs`
   `an_app_that_keeps_running_…`, `quitting_and_ending_the_session_…`. Done.
2. Toolkit: `AppConnection`, per-window globals released, `running_apps` with windowless apps.
   Done.
3. `bande-app`: the windowless loop, `reopen`, `window_closed`, `command(Option<WindowId>)`,
   menus without a window. Test: `crates/libs/app/tests/windows.rs` (close the last window and
   keep running, `Ctrl+N` without a window, a second launch reopens, `Ctrl+Q` without a window
   quits). Done.
4. Files and Settings. Done.
5. Apps repo: Terminal, Activity Monitor, Mail, Calendar, Software, Installer, template at the
   new pin. Done (apps `410a85c`, desktop pin `828e4975`).

## Gaps

- Hide for a windowless app does nothing (macOS hides the app and activates the next one).
- The Taskbar's icon menu for a windowless app has no "New Window" item; a click reopens.
- Calendar's and Activity Monitor's menus are empty without a window (Mail's too); Files and
  Terminal offer New Window, Settings its panes.
- Parked views (Settings, Mail, Calendar, Software, Installer) keep their widget trees and text
  layout caches; splitting each app's model from its view would save a few more MiB.
- Not measured: Installer (needs a package) and device memory on the mini's GPU.
