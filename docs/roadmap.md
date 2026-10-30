# Roadmap: from first draft to a desktop people keep

Written 2026-10-30 after the first end-to-end session run and an audit of every area against
macOS 15 and Windows 11. The shell, toolkit and our four apps exist and work; what is missing is
breadth where daily use starts (monitors, keyboards, third-party apps) and depth inside what we
have (Files, Settings, text, accessibility, look and sound). Each item names its decision:
**now** (this wave), **next** (following wave), **later**, or **never**. `DESIGN.md` decides
anything this file leaves open.

## How we chose

1. **Day-one blockers first.** Anything a new user hits in the first hour and cannot work
   around: a second display, a non-US keyboard, an app without a title bar, no browser.
2. **Depth before breadth.** A feature we have must work everywhere it should (DESIGN §1)
   before we add a new one.
3. **One shared system per concern.** Text editing, thumbnails, sounds, accessibility switches
   live once in the toolkit or a service, never per app.
4. **Settings only where people genuinely differ.** Every new pane is a list of real choices,
   not a mirror of macOS.

## Wave A (now): day-one blockers

| # | Area | What | Why |
|---|---|---|---|
| A1 | Third-party apps | Server-side title bars in our style for apps that ask for them (Qt, SDL, Electron, foot) instead of forcing client-side; register our FileChooser, Screenshot and ScreenCast portals (browser screen sharing); unlocked secret service at login (gnome-keyring + PAM); Qt follows dark mode and accent; GTK apps get our accent | Today such apps have **no title bar** and cannot be moved or closed; browsers and editors nag for the keyring; screen sharing fails |
| A2 | Default apps | Recommend a browser, text editor, image viewer, PDF viewer, calculator, video player, clocks in `unbound-desktop`; recommend `unbound-monitor` (built but never installed) | A fresh install cannot open a PDF or a photo |
| A3 | Multiple displays | Hotplug, per-display scale and mode, arrangement, mirroring, move windows between displays (shortcut, drag, window menu), menu bar on every display, Dock on the main one, workspaces per display; Settings ▸ Displays arrangement and resolution | Only the first connector is driven; plugging in a monitor does nothing |
| A4 | Keyboard and pointer | Input sources (xkb layouts and IMEs) with a menu bar indicator and switching shortcut; Settings ▸ Keyboard input sources and a shortcut list; Settings ▸ Trackpad & Mouse (tap to click, natural scrolling, speed, secondary click, three-finger drag later) | Anyone who types in more than one language, or wants tap-to-click, is stuck |
| A5 | Text foundations | Undo/redo in every text field; visible IME pre-edit; a multi-line `TextView`; search field; missing controls: checkbox, radio, segmented control, pop-up button, stepper, progress bar and spinner, tooltip | Typing is the most common act on a computer; CJK input is effectively unusable |

## Wave B (next): depth where people spend their day

| # | Area | What |
|---|---|---|
| B1 | Window switching | Super+Tab overlay with live thumbnails (hold to browse, Q quits, H hides), Super+` cycles one app's windows, cascade new windows, restore each app's window frame on launch |
| B2 | Files | Icon view with thumbnails (freedesktop thumbnailer spec, cached), tabs and multiple windows, recursive search over the `unbound-search` index with Recents, compress/extract (zip, tar.*), Open in Terminal, drag folders into the sidebar, Quick Look for PDF and video through thumbnailers |
| B3 | Settings breadth | General (About This Computer, Storage, Software Update via PackageKit, Date & Time, Language & Region, Login Items), Network (wired, VPN import, proxy), Battery (power modes via power-profiles-daemon, lid behaviour), Notifications per app, Displays ▸ Night Light, Sound (per-app volume, input level, alert sound) |
| B4 | Accessibility | Prove Orca end to end on every surface; zoom (magnifier in the compositor), increase contrast, reduce transparency, larger text, cursor size, sticky and slow keys; serve them through the portal so GTK/Qt apps follow |
| B5 | Capture | Screenshot window mode (Space toggles), timer, floating thumbnail you can drag into apps or open for markup (crop, arrow, text, highlight); Super+Shift+5 palette with screen recording |
| B6 | Control Center and sound | Now Playing (MPRIS) with controls, mic level and input choice, keyboard brightness; an original sound theme (alert, volume tick, screenshot, trash, plug in) through one event-sound service |

## Wave C: look, feel and finish

| # | Area | What |
|---|---|---|
| C1 | Visual coherence | An original icon set for our apps and the system icons people see daily (folders, file kinds, Trash empty/full, devices, status symbols), replacing the Adwaita mix; a design review of spacing, type scale and materials across every surface with goldens |
| C2 | Launcher | Empty query shows recent apps and files and doubles as the all-apps view (no separate Launchpad); clipboard history as a launcher mode (Super+Shift+V); emoji and symbols picker (Ctrl+Super+Space) |
| C3 | Terminal | Settings window (font, profiles from a few designed themes), split panes, visual bell and Dock badge, notify when a long command finishes in a background tab, restore windows and tabs |
| C4 | Languages | Ship Swedish, German, French and Spanish for every string; date, number and unit formats per locale; RTL mirroring later |
| C5 | First run | Setup assistant: language, keyboard, Wi-Fi, appearance, account picture; a short welcome showing the Dock, launcher and gestures |
| C6 | Activity Monitor | Process inspector (open files, threads, memory map), GPU tab, column chooser |

## Later

Network locations in Files (SMB/SFTP/WebDAV through gvfs), tags, batch rename, printers (CUPS
pane and the Print portal), users and fast user switching, an app store, Dock stacks
(Downloads), inline notification reply, colour/date/font pickers, crash reporter, backup.

## Never (for now, by design)

Dock magnification, Launchpad as a separate grid, widgets/Today view, Focus modes beyond Do Not
Disturb, text expansion, dictation, a hot corner by default.

## Decisions on open questions

- The launcher's empty query shows recents (C2); clipboard history is a launcher mode (C2).
- Notification Center slides over windows, as on macOS.
- Battery percentage: a "Show Percentage" item in the battery menu itself, no Settings switch.
- Cut stays on Super+X and in the Edit menu; item context menus stay short and drop it.
- The lock screen clock follows the locale's 12/24-hour preference.
- Empty workspaces disappear once you leave them; the overview's "+" adds one.
