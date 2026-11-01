# macOS parity

How close Bande is to macOS, area by area. **Parity** behaves like macOS 15 (or a deliberate
Linux equivalent), **basic** works but is thinner, **none** is missing. Open work is in
[`../NOTES/open.md`](../NOTES/open.md); how each feature behaves is in
[`architecture/behaviour.md`](architecture/behaviour.md).

Bande's own apps are Files, Settings, the launcher (Spotlight) and, in bande-linux/apps,
Terminal, Activity Monitor, Software, the Package Installer, Mail and Calendar. Firefox, Text
Editor, Loupe, Papers, Calculator and Showtime come preinstalled from Debian. The Taskbar sits on
the left and holds the menu bar; a setting moves the menu bar to the top.

| Area | Status | Notes |
| :--- | :--- | :--- |
| Displays: HiDPI, fractional scale, arrangement, mirroring, rotation | parity | Night Light; no colour management, HDR or custom modes |
| Toolkit: controls, menus, tables, scroll views, sheets, alerts, drag and drop | parity | no spell checking |
| Window chrome, move, resize, zoom, full screen | parity | our bar also on GTK and other server-side apps; libadwaita and Electron keep their own |
| Snapping and tiling | parity | halves and quarters, fill the other half, the zoom-button menu |
| App switcher, same-app windows, overview, App Exposé, workspaces | parity | workspaces per display, three-finger swipes; no hot corners |
| Taskbar: kept and running apps, Trash, icon menus, badges, drops | parity | the Dock and the menu bar in one bar |
| Menu bar app menus and menu search | parity | from every `bande-app` app and dbusmenu exporters |
| Launcher: apps, settings, files, calculator, recents, clipboard history, emoji | parity | file names only, not contents |
| Notifications: banners, Notification Center, Do Not Disturb | parity | no schedules, no inline reply, nothing on the lock screen |
| Control Center, status items, volume and brightness HUD, Now Playing | parity | |
| Desktop: wallpaper, icons, menus | parity | |
| Lock screen, login window | basic | no user picker, accessibility or power menus at login |
| Files | basic | icons, list and columns, Quick Look, Get Info, tabs, batch rename, settings; no network locations or tags |
| Settings | basic | grouped panes with search; no Users pane, printers or custom shortcuts |
| Terminal, Activity Monitor | basic | apps repo; Terminal has tabs, splits and profiles |
| Software, Package Installer, software updates | basic | live updates, offline only for reboot-class packages; Debian packages only; no Flatpak yet |
| Screenshots, recording, Markup | parity | |
| Force Quit, Not Responding, low-memory window | parity | in the compositor, works when the shell hangs |
| Sleep and wake | basic | display sleep works; system sleep is off on the M2 mini until s2idle is reliable |
| Accessibility: screen reader, zoom, contrast, motion, text size, sticky/slow/bounce keys | basic | no colour filters or on-screen keyboard |
| Input methods (CJK) | basic | work in apps; not offered as input sources in Settings |
| First-run setup | none | |
