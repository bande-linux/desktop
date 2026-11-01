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

## Efficiency

The audit (2026-10-30) found the libraries lean and the weight in how we use them: about 1.9 GB per session on the VM, mostly one GPU device and font database per shell process. Quick wins (GPU atlas sizing, one device per process, damage tracking, a shipping profile with LTO, dependency trims, stale daemons) are in progress; the structural win is E7. Measurements live in `docs/architecture/performance.md`.

## Wave D (now, top priority): Datum and the combined bar

Adopted by the owner on 2026-10-30: the design language [Datum](design/datum.md) (with rounded-square app icons and the richer
icon style of the bar mockups, per the owner's amendment in §10) and the [combined bar](design/features/bar.md) with app menus as a cascade
from the app header. They replace the macOS-looking first draft. Two tracks run in parallel;
feature work in waves A–B continues, using design tokens only (no hard-coded colours, radii or
fonts) so it restyles for free.

### Track D: Datum, from the tokens outwards

| # | Step | Contents | Depends on |
|---|---|---|---|
| D1 | Tokens and window chrome | `bande-design` crate: OKLCH palette generator with 8 accents, radii (4/6/8/10/14, concentric rule), spacing and control sizes, type roles, elevation, contrast tests (WCAG + APCA) that fail CI; fonts (Inter features cv05/cv08/ss07/ss08/tnum, Manrope, JetBrains Mono); the 48 pt frame header with flush-left title and the left control pill in the toolkit and the compositor's server-side frame; corner radius 14; frame/sheet split | — |
| D2 | Controls | Every widget on the tokens: buttons, fields, switches, checkbox, square radio, segmented, pop-up, stepper, fader slider, segmented progress, tooltips, tables with gliding selection, menus with shortcuts written as words, alerts with the footer tray, sheets, scroll bars | D1 |
| D3 | Icons | The Bande icon theme: rounded-square app icons in the style of the owner-picked bar mockup for our apps (and replacements for the default third-party apps), sheet+label file types, stone folders with an accent tab, the 16 pt symbol set; reproducible generators | — |
| D4 | Apps | Files, Settings, Terminal (JetBrains Mono, accent-derived ANSI), Activity Monitor on frame and sheet with the toolbar in the header | D2 |
| D5 | Shell surfaces | Launcher, notifications, Control Center, lock, greeter, screenshot, overview and HUDs on the tokens (opaque, scrim + blur only where Datum allows) | D2 |
| D6 | Everyone else and choice | GTK and Qt receive the palette (gtk.css accent and named colours, Qt colour scheme); Settings ▸ Appearance: Light/Dark/Auto, 8 accent swatches, tinted frame, Increase Contrast, larger text | D2 |

### Track E: the combined bar and one shell process

| # | Step | Contents | Depends on |
|---|---|---|---|
| E1 | One bar process | `bande-bar` hosts today's Dock and menu bar surfaces with no visual change; shared models move to `crates/libs`; the compositor starts one client | multiple displays on main |
| E2 | Vertical bar | Bottom group: Trash, workspaces, tray grid with overflow, the Control Center button (2×2), clock with unread dot; launcher mark at the top corner (click: launcher, right-click: system menu) | E1 |
| E3 | App header and cascade | Active app header; one cascade menu with the app row and each menu as a row, filtered by typing; `Super+F2`; right-click everywhere; fallback content for apps without menus | E2 |
| E4 | Menu search | `Shift+Super+/` card that finds any menu item with its path and shortcut | E3 |
| E5 | Default and settings | Combined left bar (the taskbar) is the default for new installs; Settings ▸ Taskbar (was Settings ▸ Bar): position, size, show names, auto-hide, separate menu bar; config migration from `dock {}` | E3 |
| E6 | Expanded, horizontal, panels | Pinned expanded mode with names and window rows; bottom/top layouts with inline menu titles; Control Center and Notification Center open beside their item on any edge; full-screen reveal; per display | E5 |
| E7 | One shell process | Desktop surface, notifications, Control Center and launcher hosted in the bar's process (one Wayland connection, one GPU device, one font database); lock screen and greeter stay separate | E5 |
| E8 | Third-party menus | `org_kde_kwin_appmenu` so Qt/KDE apps' menus appear in the header | E3 |

Datum styles the bar from E2 on; E1 changes nothing visible.

## Wave A: day-one blockers

| # | Area | What | Why |
|---|---|---|---|
| A1 | Third-party apps | Server-side title bars in our style for apps that ask for them (Qt, SDL, Electron, foot) instead of forcing client-side; register our FileChooser, Screenshot and ScreenCast portals (browser screen sharing); unlocked secret service at login (gnome-keyring + PAM); Qt follows dark mode and accent; GTK apps get our accent | Today such apps have **no title bar** and cannot be moved or closed; browsers and editors nag for the keyring; screen sharing fails |
| A2 | Default apps | Recommend a browser, text editor, image viewer, PDF viewer, calculator, video player, clocks in `bande-desktop`; recommend `bande-monitor` (built but never installed) | A fresh install cannot open a PDF or a photo |
| A3 | Multiple displays | Hotplug, per-display scale and mode, arrangement, mirroring, move windows between displays (shortcut, drag, window menu), menu bar on every display, the taskbar on the main one, workspaces per display; Settings ▸ Displays arrangement and resolution | Only the first connector is driven; plugging in a monitor does nothing |
| A4 | Keyboard and pointer | Input sources (xkb layouts and IMEs) with a menu bar indicator and switching shortcut; Settings ▸ Keyboard input sources and a shortcut list; Settings ▸ Trackpad & Mouse (tap to click, natural scrolling, speed, secondary click, three-finger drag later) | Anyone who types in more than one language, or wants tap-to-click, is stuck |
| A5 | Text foundations | Undo/redo in every text field; visible IME pre-edit; a multi-line `TextView`; search field; missing controls: checkbox, radio, segmented control, pop-up button, stepper, progress bar and spinner, tooltip | Typing is the most common act on a computer; CJK input is effectively unusable |

## Wave B (next): depth where people spend their day

| # | Area | What |
|---|---|---|
| B1 | Window switching | Super+Tab overlay with live thumbnails (hold to browse, Q quits, H hides), Super+` cycles one app's windows, cascade new windows, restore each app's window frame on launch |
| B2 | Files | Icon view with thumbnails (freedesktop thumbnailer spec, cached), tabs and multiple windows, recursive search over the `bande-search` index with Recents, compress/extract (zip, tar.*), Open in Terminal, drag folders into the sidebar, Quick Look for PDF and video through thumbnailers |
| B3 | Settings breadth | General (About This Computer, Storage, Software Update via PackageKit, Date & Time, Language & Region, Login Items), Network (wired, VPN import, proxy), Battery (power modes via power-profiles-daemon, lid behaviour), Notifications per app, Displays ▸ Night Light, Sound (per-app volume, input level, alert sound) |
| B4 | Accessibility | Prove Orca end to end on every surface; zoom (magnifier in the compositor), increase contrast, reduce transparency, larger text, cursor size, sticky and slow keys; serve them through the portal so GTK/Qt apps follow |
| B5 | Capture | Screenshot window mode (Space toggles), timer, floating thumbnail you can drag into apps or open for markup (crop, arrow, text, highlight); Print palette with screen recording |
| B6 | Control Center and sound | Now Playing (MPRIS) with controls, mic level and input choice, keyboard brightness; an original sound theme (alert, volume tick, screenshot, trash, plug in) through one event-sound service |

## Wave C: look, feel and finish

| # | Area | What |
|---|---|---|
| C1 | Visual coherence | Moved to wave D (Datum D3 and the design review in D4–D5) |
| C2 | Launcher | Empty query shows recent apps and files and doubles as the all-apps view (no separate Launchpad); clipboard history as a launcher mode (Super+V); emoji and symbols picker (Super+.) |
| C3 | Terminal | Settings window (font, profiles from a few designed themes), split panes, visual bell and taskbar badge, notify when a long command finishes in a background tab, restore windows and tabs |
| C4 | Languages | Ship Swedish, German, French and Spanish for every string; date, number and unit formats per locale; RTL mirroring later |
| C5 | First run | Setup assistant: language, keyboard, Wi-Fi, appearance, account picture; a short welcome showing the taskbar, launcher and gestures |
| C6 | Activity Monitor | Process inspector (open files, threads, memory map), GPU tab, column chooser |

## Later

Network locations in Files (SMB/SFTP/WebDAV through gvfs), tags, batch rename, printers (CUPS
pane and the Print portal), users and fast user switching, an app store, taskbar stacks
(Downloads), inline notification reply, colour/date/font pickers, crash reporter, backup.

## Never (for now, by design)

Taskbar magnification, Launchpad as a separate grid, widgets/Today view, Focus modes beyond Do Not
Disturb, text expansion, dictation, a hot corner by default.

## Decisions on open questions

- The launcher's empty query shows recents (C2); clipboard history is a launcher mode (C2).
- Notification Center slides over windows, as on macOS.
- Battery percentage: a "Show Percentage" item in the battery menu itself, no Settings switch.
- Cut stays on Ctrl+X and in the Edit menu; item context menus stay short and drop it.
- The lock screen clock follows the locale's 12/24-hour preference.
- Empty workspaces disappear once you leave them; the overview's "+" adds one.
