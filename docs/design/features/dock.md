# Dock
Status: superseded in part by the [combined bar](bar.md) (owner, 2026-10-30): in the default combined mode the bar also holds the app menus, status items, tray and clock. The rules below still hold for the app area and for "separate menu bar" mode.

## Purpose
The Dock (`unbound-dock`) is where apps are started, found again and quit. It shows the apps a
person keeps, the apps that are running, and the Trash, and every icon answers a left click, a
right click, a drag and a dropped file. It does one job well: it is not a window list, a clock,
a system tray or a launcher grid, all of which have their own homes (menu bar, launcher,
Exposé).

Modifier names below follow the decided mapping: Super acts as Command. Alt is assumed to stand
in for Option (not yet recorded in `docs/architecture/overview.md`).

## Reference survey

| Feature | macOS | GNOME / KDE / Windows | Note |
|---|---|---|---|
| Kept and running apps | Kept apps, then up to three recent apps behind a divider, then files and folders, then Trash [1] | Windows and KDE mix pinned and running in one strip | The divider separates apps from documents [1]. |
| Running indicator | Dot under open apps; a setting turns it off [2] | Windows: underline, longer for focused app | A dot is the quietest working signal. |
| Launch feedback | Icons bounce while apps open; a setting turns it off [2] | | |
| Hover name | Name label above the icon (unverified: not on the fetched pages) | Windows shows window thumbnails on hover | |
| Magnification | Off/on with a size slider; Ctrl+Shift toggles it while held [2][4] | Not offered by GNOME, KDE or Windows (unverified) | Makes targets move under the pointer. |
| Icon menu | Control-click: window list, Options (Keep in Dock, Open at Login, Show in Finder), Quit, Force Quit [1]; apps add items above the system ones [5, unverified: page did not render] | Windows: jump lists with recent files and app tasks; now also on hover [9] | Desktop entry `Actions` give the Linux equivalent of app tasks [12]. |
| Modifier clicks | Command-click shows in Finder; Option-click hides the current app; Option-Command-click hides all others [1] | | |
| Reorder | Drag an item to a new place [1] | All three allow it | |
| Add | Drag an app left of the divider, files and folders right of it [1] | Windows: pin from context menu | |
| Remove | Drag out until Remove appears; only the alias goes [1] | Windows and KDE: Unpin in the menu | Drag-out is fast but easy to trigger by accident. |
| Folders and stacks | Folder shown as folder or stack; fan, grid or list; sort by name, date added, date created [3]. Downloads stack by default [1] | | Fan was dropped in some versions [11]. |
| Trash | Fixed at the end of the Dock [1] | GNOME and Windows keep Trash on the desktop or in the file manager | |
| Minimized windows | Into a Dock area, or into the app icon [2] | Windows and KDE: into the task button | |
| Badges | Red badge means something needs action [1] | KDE and Ubuntu: `com.canonical.Unity.LauncherEntry` count, progress, urgent [6][7]; Windows: badges, progress bar, flashing button [8][10] | The LauncherEntry D-Bus API is what Linux apps already speak. |
| Attention | Icon bounces (unverified: not on the fetched pages) | Windows flashes the button, 7 times by default [10]; Focus sessions suppress it | |
| Auto-hide | Setting; Option-Command-D toggles [2][4] | All three | |
| Size and position | Size slider, left/bottom/right [2]; drag the separator to resize [1] | Windows: bottom only | |
| Multiple displays | On all displays when displays have separate Spaces, else main display only [2] | | |
| Drop file on icon | Opens the file in that app [1] | | |
| Keyboard | Fn-Control-F3 focuses the Dock, arrows move, Return opens [1] | | |

## Proposed Unbound feature set

### Now (essential)

**Layout** — One strip in three groups divided by thin lines: kept apps, then running apps that
are not kept, then Trash. The clock is removed; the menu bar owns the time.

**Running indicator** — A dot beside each running app's icon on the edge side. One dot whatever
the window count. No setting to hide it.
Motion: the dot fades and scales in when the app's first window maps and out when the app exits.

**Launch feedback** — The icon gets its dot immediately and pulses gently (not a bounce: our own
look) until the app maps a window or a 10 s timeout passes. A running app's icon that is not kept
slides in at the end of its group as the launch starts, not when the window appears.
Motion: pulse in place; neighbours slide apart to make room.

**Hover label** — The app's name in a small label beside the icon, on the screen side (right of
the icon for a left Dock). The first label waits about 400 ms; moving to the next icon swaps the
label instantly. Pressing the mouse hides it.
Inputs: hover.
Motion: fade in; the label glides between icons rather than reappearing.

**Click to open or activate** — Not running: start it. Running: bring all its windows forward
with the frontmost focused; if every window is minimized, restore the latest (M15 behaviour).
Inputs: left click opens or activates; Super+click shows the app in Files; Alt+click activates
and hides the previously active app; Alt+Super+click hides all others [1].

**Icon menu** — One menu per icon, in this order:
1. Open windows by title, the focused one checked, minimized ones marked (choosing one restores
   and focuses it).
2. The app's desktop entry actions (for example New Window, New Private Window) [12].
3. Keep in Dock (checkmark), Open at Login (checkmark), Show in Files.
4. Hide, Quit.
Quit becomes Force Quit while Alt is held, and is shown as Force Quit without Alt when the
watchdog marks the app unresponsive. No "Options" submenu: three checkmarks do not need one.
Inputs: right click, or press and hold for 500 ms; Menu key or Shift+F10 when the Dock has
keyboard focus.
Motion: menu scales out from the icon; closes on Escape or a click elsewhere.

**Keep and remove** — Keep in Dock from the menu, or drag an app from Files or the launcher into
the kept group. Remove by unchecking Keep in Dock, or by dragging the icon out of the Dock: past
about one icon width a "Remove" label appears, and releasing there removes it. A running app
dragged out stays in the running group. Kept apps are written to `config.kdl`.
Inputs: drag in, drag out, right click.
Motion: neighbours close the gap; the removed icon shrinks and fades at the pointer.

**Reorder** — Drag any icon within its group; others slide aside live. Dragging a running app
into the kept group keeps it.
Inputs: drag (starts after 4 px of movement so clicks stay clicks).
Motion: spring slide, interruptible mid-drag.

**Trash** — Fixed last item. Empty and full icons. Click opens the Trash in Files; dropping files
moves them to the Trash; dropping a removable volume ejects it (macOS behaviour, unverified on
the fetched pages). Menu: Open, Empty Trash (asks first).
Inputs: left click, right click, drop.

**Drop files on app icons** — Dragging files over an icon highlights it if the app's desktop
entry lists a matching `MimeType`, and dims it otherwise; releasing opens the files in that app,
starting it if needed. Hovering over a running app's icon for about 1 s while dragging brings its
windows forward so the drop can continue into a window (unverified as macOS behaviour).
Inputs: drag over, drop.
Motion: accepting icon grows slightly.

**Badges, progress and attention** — Read `com.canonical.Unity.LauncherEntry` updates on D-Bus
(count, progress, urgent) as KDE does [6][7]. Count: a small badge on the icon corner,
"99+" above 99, hidden for zero or negative values. Progress: a thin bar across the icon's
lower edge. Attention (urgent flag, or an `xdg-activation-v1` request without a valid token
[13]): the icon pulses twice and keeps a ring until the app is activated. Do Not Disturb
suppresses the pulse, not the ring.
Motion: badge pops in with a small overshoot; progress animates between values.

**Minimize into the app icon** — Minimized windows go into their app's icon; they are listed in
the icon menu and a click on the icon restores the latest. No separate minimized-window area.
Motion: the window shrinks toward the icon (M5 genie-like effect in our own style).

**Keyboard reach** — A shortcut focuses the Dock (proposed Super+Alt+K, unverified as free);
Up/Down (or Left/Right on a horizontal Dock) move, Return opens, Menu key opens the menu, Escape
returns focus. Super+Alt+D toggles auto-hide [4]. Every icon is an AccessKit button with its
name, running state, badge and progress.

**Size, position, auto-hide** — Three settings in Desktop & Dock: icon size slider, edge
(left default, bottom, right, top), automatically hide. With auto-hide, the Dock slides in when
the pointer touches its edge and slides out after the pointer leaves. Dragging the divider
before Trash resizes the Dock directly [1].
Motion: slide, interruptible: touching the edge again mid-hide reverses it.

### Later (power features worth having)

**Folders as stacks** — Folders kept right of the app group. Click opens one view, a scrollable
grid with "Open in Files" at the bottom; sorted by date added by default, changeable in the
folder's menu [3]. A Downloads stack is not added by default (see open questions).

**Window previews in the icon menu** — Small thumbnails beside window titles, from the
compositor's existing window textures.

**Show the app's windows** — Clicking the icon of the already-frontmost app opens an Exposé of
just its windows (macOS does this from Control-Down Arrow [4]; the click mapping is ours).

**Middle click opens a new window** — Runs the desktop entry's New Window action when it exists
(GNOME and KDE convention, unverified).

**Multi-display follow** — Dock on the primary display; pushing the pointer against the Dock
edge of another display moves it there (macOS behaviour, unverified on the fetched pages).

**Recent documents in the icon menu** — Per app, from `recently-used.xbel`, like Windows jump
lists [9].

### Never (left out)

**Magnification** — Moving targets trade precision for show; a well-chosen size plus hover
labels is simpler and every major Linux and Windows dock does without it.

**Recent and suggested apps** — Running apps already appear; a second automatic group adds a
concept that reshuffles the Dock without being asked.

**Separate minimized-window area** — Two places to find a window is one too many; the icon menu
lists minimized windows.

**Window thumbnails on hover** — Hover must stay cheap and calm; the menu and Exposé cover it.

**Indicator and launch-animation toggles** — Settings only where people genuinely differ; a dot
and a short pulse bother nobody enough to need switches.

**Fan and list stack styles, per-stack display options** — One stack view done well.

**Clock, tray icons, workspace switcher in the Dock** — They belong to the menu bar and
Mission Control; the Dock stays about apps.

**Arbitrary files kept in the Dock** — Folders only (Later); individual documents belong in
Files favourites or Recents.

## Open questions for the owner

1. **Magnification: never, or an off-by-default setting?** Recommendation: never. It is the
   most visible macOS imitation and the least useful.
2. **Downloads stack by default?** macOS ships one [1]. Recommendation: no stack by default;
   Downloads is in the Files sidebar, and download progress shows as the browser's badge and
   progress on its icon.
3. **Removal by dragging out.** Recommendation: keep it, but only past a clear threshold with the
   "Remove" label, plus the menu checkmark as the obvious way.
4. **Click on the frontmost app's icon.** macOS does nothing visible (unverified); Windows
   minimizes. Recommendation: nothing in Now, app Exposé Later; never minimize, which surprises.
5. **Alt as Option.** The modifier clicks and Force Quit assume Alt plays Option's role.
   Recommendation: record that in section 1.7 alongside Super as Command.

## Sources
1. Apple, Use the Dock on Mac: https://support.apple.com/guide/mac-help/use-the-dock-on-mac-mh35859/mac
2. Apple, Change Desktop & Dock settings on Mac: https://support.apple.com/guide/mac-help/change-desktop-dock-settings-mchlp1119/mac
3. Apple, Use folders in the Dock on Mac: https://support.apple.com/guide/mac-help/use-folders-in-the-dock-mchl231f08fb/mac
4. Apple, Mac keyboard shortcuts: https://support.apple.com/en-us/102650
5. Apple HIG, Dock menus (page did not render for fetching): https://developer.apple.com/design/human-interface-guidelines/dock-menus
6. Ubuntu wiki, Unity Launcher API: https://wiki.ubuntu.com/Unity/LauncherAPI
7. KDE Plasma task manager, smart launcher backend: https://lxr.kde.org/source/plasma/plasma-desktop/applets/taskmanager/plugin/smartlaunchers/smartlauncherbackend.cpp
8. The Old New Thing, flashing the taskbar button: https://devblogs.microsoft.com/oldnewthing/20130211-00/?p=5283
9. Windows Latest, jump lists on hover: https://windowslatest.com/2024/07/24/windows-11-taskbar-now-shows-jump-list-on-hover-gets-new-android-integration-with-continuity/amp
10. Geek Rewind, taskbar flashing and Focus sessions: https://geekrewind.com/how-to-enable-or-disable-hide-flashing-on-taskbar-apps-with-focus-session-in-windows-11/
11. Wikipedia, Stacks (Mac OS): https://en.wikipedia.org/wiki/Stacks_(Mac_OS)
12. freedesktop.org, Desktop Entry Specification (Additional application actions): https://specifications.freedesktop.org/desktop-entry-spec/latest/
13. Wayland, xdg-activation-v1: https://wayland.app/protocols/xdg-activation-v1
