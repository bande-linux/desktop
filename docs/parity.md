# macOS parity

How close Unbound is to modern macOS, feature by feature. Each row names the milestone that
brings it to parity (`NOTES/backlog.md`); the status is updated when a milestone lands and is
checked live on the dev VM, not only in tests.

Status: **none** (missing), **basic** (works, far from macOS), **parity** (behaves like macOS 15).

Default apps follow a hybrid plan: Finder, System Settings, Terminal, Activity Monitor and
Spotlight are ours; Firefox, GNOME Text Editor, Loupe, Papers, Calculator, Showtime and Clocks are
preinstalled. The Dock sits on the left by default, the menu bar on top.

## Foundations

| Feature | Status | Milestone |
|---|---|---|
| HiDPI and fractional scaling | basic (compositor and toolkit scale to output) | M1 |
| Menus and context menus in the toolkit | none | M1 |
| Scroll views, lists, tables, icon grids | none | M1 |
| Window title bar with traffic lights, move and resize | none (no chrome) | M1 |
| Drag and drop | none | M1 |
| Icons from the icon theme in the toolkit | basic (panel, launcher) | M1 |

## Desktop

| Feature | Status | Milestone |
|---|---|---|
| Wallpaper, choosable in Settings | none (flat colour) | M2 |
| Desktop icons (files, volumes), selection, double-click to open | none | M2 |
| Desktop right-click menu (New Folder, Get Info, Change Wallpaper) | none | M2 |
| Lock screen and greeter wallpaper | none | M2 |

## Discovery and launching

| Feature | Status | Milestone |
|---|---|---|
| Launchpad (all apps grid) | none | M3 |
| Spotlight (apps, files, settings, calculator) | basic (app names only) | M3 |
| Dock: pinned and running apps, Applications stack, Trash, icon menus, reordering | basic | M3 |
| Apple menu (About, Settings, Force Quit, Sleep, Restart, Shut Down, Lock, Log Out) | none | M3 |

## Apps

| Feature | Status | Milestone |
|---|---|---|
| Browser, text editor, image and PDF viewer, calculator, video, clocks preinstalled | none | M4 |
| GTK apps follow accent colour and dark mode | basic (sandboxed only) | M4 |
| Default app per file type (Open With) | none | M4 |
| App menus in the menu bar | none (app name only) | M4 |
| Finder: sidebar, icon/list/column/gallery views, Get Info, Trash, Quick Look | basic (columns, PNG and text Quick Look) | M7 |
| System Settings: all panes with search | basic (3 panes) | M7 |
| Terminal with tabs | basic | M7 |
| Activity Monitor | none | M7 |

## Windows

| Feature | Status | Milestone |
|---|---|---|
| App switcher overlay (Super+Tab) and same-app windows (Super+`) | basic (switch without overlay) | M5 |
| Mission Control and Exposé | done (overview with workspace strip, titles, filter, drag between workspaces; App Exposé) | M5 |
| Spaces (workspaces, full-screen spaces, three-finger swipes) | done (one output) | M5 |
| Minimize to Dock, zoom, spring animations | basic (minimize, no animation) | M5 |
| Snapping and tiling to halves and quarters | none | M5 |

## Status and notifications

| Feature | Status | Milestone |
|---|---|---|
| Notification banners | none (model only) | M6 |
| Notification Center | none | M6 |
| Menu bar status items (Wi-Fi, battery, sound, Bluetooth, tray) | none | M6 |
| Control Center (Wi-Fi, Bluetooth, Do Not Disturb, sound) | basic | M6 |
| Volume and brightness HUD | basic (volume) | M6 |

## Polish

| Feature | Status | Milestone |
|---|---|---|
| Animations for open, close, menus | none | M8 |
| Coherent icon set for our apps | basic | M8 |
| First-run setup assistant | none | M8 |
| Screen reader access in every app | basic (shell only) | M8 |
| Input methods (CJK) | basic (compositor and text fields) | done |
| Lock screen, greeter, screenshots | basic | done |
