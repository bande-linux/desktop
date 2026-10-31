# The combined bar: Dock and menu bar as one bar

Status: adopted by the owner on 2026-10-30 (app menus: cascade from the app header, option A). Implementation order: `docs/roadmap.md`, wave D.
Mockups: `../bar/` (SVG + PNG; regenerate with
`python3 generate.py <outdir>` and any SVG renderer; the PNGs were drawn with resvg 0.49, the
version the workspace pins).

## 1. The idea in one paragraph

One bar, on the left edge by default, holds everything that today lives in the Dock and the menu
bar, in one fixed order from the top corner to the bottom corner: **launcher, the active app and
its menus, the apps, Trash, workspaces, tray, Control Center, clock and notifications**. It is 64 pt
wide by default and can be pinned open to 248 pt, where everything gets its name (a sidebar). The
same model is drawn horizontally at the bottom or top. People who want the classic split keep a
separate top menu bar with one switch. App menus live behind the active app's header as one
cascading menu that filters as soon as you type (no visible field), the same search also opens with `Shift+Super+/` from
anywhere, and in the expanded bar the menu titles are listed as rows.

## 2. Prior art: what worked and what failed

| System | What it did | What we take | Sources |
|---|---|---|---|
| **Haiku Deskbar** (BeOS 1996 onward) | One panel holding the system menu ("leaf"), tray and clock ("replicant tray"), and the running apps, each app expandable to its windows. Vertical by default in a corner, can be expanded (per-app window lists inline), mini (one line) or horizontal along an edge. Users still debate its modes on the forums 25 years later; mini mode confused people looking for how to turn it on. | Closest precedent: one bar can carry launcher, apps, windows and status without crowding when ordered top-to-bottom. Window lists inline under an app ("expando") are the vertical-tabs idea. Lesson: too many modes; we ship two widths, not four layouts. | [Haiku user guide: Deskbar](https://haiku-os.org/docs/userguide/en/deskbar.html), [forum: mini mode](https://discuss.haiku-os.org/t/the-new-deskbar-mini-mode/9388), [forum: settings polls](https://discuss.haiku-os.org/t/what-are-your-deskbar-settings-polls/9344) |
| **NeXTSTEP** | Dock as a vertical column at the right edge; app menus as a vertical, tear-off menu panel at the top left of the screen, each submenu cascading to the right. | Vertical menus are not an oddity: NeXT shipped them for a decade and they read well, because menu items are words that read left to right in rows. The cascade-to-the-side is the natural vertical form. | [NeXTSTEP User Interface Guidelines, ch. "Menus"](https://www.nextcomputers.org/files/manuals/nd/UserInterface/) (archive), [Wikipedia: NeXTSTEP](https://en.wikipedia.org/wiki/NeXTSTEP) |
| **Ubuntu Unity** (2010–2017) | Left launcher (≈64 px with 48 px icons) + top panel with global menu + HUD (Alt tap, search every menu item). Rationale: wide screens have width to spare, 1366×768 netbooks have no height to spare. Complaints: launcher could not move (bottom came only in 16.04), global menu far from window on large screens (14.04 added "locally integrated menus"), menus hidden until hover hurt discoverability, HUD alone was too hidden; Shuttleworth admitted hiding menus in testing was too aggressive. | Left edge is right for 16:9/16:10. Menus must stay visible (titles, not only on hover); search is a powerful *layer*, never the only path. Position must be a choice from day one. | [Canonical: Introducing the HUD](https://canonical.com/blog/introducing-the-hud-say-hello-to-the-future-of-the-menu), [TechCrunch on HUD](https://techcrunch.com/2012/01/25/mark-shuttleworth-unveils-new-head-up-display-for-ubuntu-12-04), [unity-design list on left placement](https://archive.lists.launchpad.net/unity-design/msg06320.html) |
| **Windows taskbar** | Start, search, apps, tray, clock in one bar since 1995; vertical mode until Windows 10, removed in Windows 11 ("disrupts animations"), returned in 2026 Insider builds (left/right/top, smaller size, labelled buttons when "never combine"). Tray overflow chevron. Vertical Windows 10 taskbar was famous for wasting width (tray icons in a grid, wide clock). | One bar for launcher + apps + status is a proven daily-use model at enormous scale. Vertical people are a loud minority and leaving them out cost Microsoft years of complaints. Tray needs an overflow. Labelled vertical buttons are popular on wide screens but get unwieldy with many windows, so we label apps, not every window. | [Windows Latest, May 2026](https://windowslatest.com/2026/05/16/microsoft-rolls-out-windows-11s-movable-and-smaller-taskbar-5-years-after-killing-it-hands-on-experience), [Pureinfotech, K2 movable taskbar](https://pureinfotech.com/windows-11-taskbar-position-top-left-right-k2/), [How-To Geek on Windows 11 removal](https://www.howtogeek.com/737198/windows-11-wont-let-you-move-the-taskbar-but-it-should/) |
| **ChromeOS shelf** | Launcher at the start corner, apps, then a status area whose one button (time, Wi-Fi, battery together) opens Quick Settings with user, lock, power. Shelf can be left, bottom or right; auto-hide. | Status icons grouped into **one** Quick Settings button is simpler than five tiny targets, and puts session actions (lock, power) in that panel. | [ChromeOS help: shelf](https://support.google.com/chromebook/answer/9032426), [ChromeOS help: quick settings](https://support.google.com/chromebook/answer/2589437) |
| **KDE Plasma** | Any panel on any edge, vertical panels supported; Global Menu applet (horizontal) and an "app menu button" variant that opens the whole menu from one button, good for vertical panels. Wayland global menu via `org_kde_kwin_appmenu` + dbusmenu; GTK apps mostly do not export on Wayland. | The "menu button that opens the app menu as a cascade" exists and works in vertical panels. Third-party menu export on Wayland is partial, so the design must be graceful without it. | [Plasma appmenu protocol](https://smithay.github.io/wayland-rs/wayland_protocols_plasma/appmenu/client/__interfaces/static.ORG_KDE_KWIN_APPMENU_INTERFACE.html), [KDE Phabricator D3089](https://phabricator.kde.org/D3089), [Arch forum on GTK menus on Wayland](https://bbs.archlinux.org/viewtopic.php?id=286389) |
| **GNOME top bar** | Activities, clock in the centre, one system menu at the right. Retired the app menu in 3.32 for poor usability; status grouped into one menu. | Grouped system status = one target. Lesson from the app menu retirement: a single half-empty app menu that only some apps use is worse than none; ours must always have content (the standard app items) even for apps that export nothing. | [GNOME app menu retirement](https://gitlab.gnome.org/GNOME/Initiatives/-/issues/4), [aday's blog](https://blogs.gnome.org/aday/?p=2749) |
| **COSMIC** (System76) | Separate panel and dock, each placeable on any edge, each with applets; "extend to edges" or floating. | Confirms people move bars; also shows two bars of applets means two places to configure. One bar keeps configuration in one place. | [COSMIC panel/dock settings](https://system76.com/cosmic) |
| **macOS menu bar** | App menus + status items share one strip; on notched MacBooks status items silently vanish behind the notch; third-party tools (Bartender, Ice) exist only to manage crowding. Apple: menus win over extras when space is short. | A horizontal bar runs out of room on laptops; a vertical bar has more room for status items but must have an explicit overflow. Never hide items without a visible "more". | [9to5Mac: notch hides status items](https://9to5mac.com/2021/10/27/macbook-pro-notch-problem-status-items/), [Michael Tsai roundup](https://mjtsai.com/blog/2021/11/24/the-macbook-pro-notch), [Jesse Squires: notch fixes](https://www.jessesquires.com/blog/2023/12/16/macbook-notch-and-menu-bar-fixes/) |
| **Spotlight / Raycast, macOS Help menu search** | Help ▸ Search finds any menu item and points at it; Raycast and Spotlight search actions. | Menu search belongs to every app, for free, from the menu model we already export. Show the path ("Edit › Find") so search also teaches where things are. | [Apple: Help menu search](https://support.apple.com/guide/mac-help/use-help-mh35846/mac), [Raycast](https://www.raycast.com/) |
| **VS Code activity bar, Slack/Discord/Teams rails** | 48 px icon rail at the left with hover tooltips, account and settings at the bottom; widely accepted. | A 48–72 pt icon rail with a bottom cluster for "you and settings" is a pattern millions use daily; tooltips/labels on hover are enough at that width. | [VS Code UI](https://code.visualstudio.com/docs/getstarted/userinterface) |
| **Arc browser** (vertical sidebar) | Tabs as a vertical, labelled sidebar that can be hidden; widely praised on wide screens, but people on small laptops hide it. | Pinned expanded mode with names is great on wide screens, wrong as a default on 13". | [Arc help: sidebar](https://resources.arc.net/hc/en-us/articles/19228064149143-Sidebar) |
| **iPadOS dock / Stage Manager** | Dock reveals from the edge over full-screen apps; recent apps strip at the left in Stage Manager. | Reveal-from-edge over full screen with a deliberate gesture; left strip for app switching. | [Apple: Stage Manager](https://support.apple.com/guide/ipad/ipad1240f36f/ipados) |

Sources fetched or searched in this session: Haiku, Unity/HUD, Windows 2026 taskbar, KDE appmenu, GNOME app menu retirement, macOS notch. The NeXTSTEP, ChromeOS, COSMIC, Apple Help, VS Code, Arc and Stage Manager links are cited from knowledge and were not re-fetched.

### Principles extracted

1. **Wide screens have width to spare, not height.** 16:9, 16:10 and 21:9 screens lose least by
   giving up a column: a 64 pt left bar costs 3.3 % of 1920 width; the current 24 pt menu bar +
   a bottom Dock cost 7.4 % of 1080 height, where text and documents need it.
2. **Fitts's law: edges and corners.** A bar whose targets reach the screen edge has infinitely
   deep targets; the two corners of the bar's edge are the best targets on screen. Put the two
   most-used entries there: launcher (top corner) and clock/notifications (bottom corner).
   The left edge is also on the near side of the window controls, which are at the top left of
   every window (no long pointer travel between menu and window).
3. **Menus must be visible and discoverable.** Unity and GNOME both show that hiding menus
   (hover-only, search-only, app menu nobody opens) fails. A clearly marked, always-present entry
   that opens the full menu is the floor; search is the ceiling.
4. **Do not rotate text.** Vertical labels read badly; every label in the bar is horizontal. The
   narrow bar uses icons + short captions; names appear on hover or in the expanded width.
5. **Density by grouping, not shrinking.** Four status glyphs as one button beats four 16 pt
   targets; tray items in a 2-column grid; "+N" and "•••" for overflow, never silent hiding.
6. **Two sizes, not four layouts.** Haiku's four Deskbar modes are a cautionary tale. Collapsed and
   expanded, on any edge, plus "separate menu bar" for those who want it.

## 3. The design

### 3.1 Order (left edge; top to bottom)

| # | Region | Collapsed (64 pt) | Expanded (248 pt) | Click | Right click |
|---|---|---|---|---|---|
| 1 | **Mark / launcher** (top corner) | Bande mark, 64×52 cell touching the corner | Mark + search field "Search  ⟨Super Space⟩" | Opens the launcher (same as `Super+Space`; centred card as today) | System menu: About This Computer, System Settings…, Force Quit…, Sleep, Restart…, Shut Down…, Lock Screen, Log Out *user*… |
| 2 | **App header** | Active app icon 24 pt, bold 11 pt name caption (ellipsised), `›` | Icon, bold app name, menu glyph; below it the app's menu titles as rows (File, Edit, View, …) | Opens the **app menu** (§3.3) | Same menu (it is the app's) |
| 3 | **Apps** | Kept apps, hairline, running apps not kept. Icon 40 pt in 48 pt cells. Running: 4 pt dot at the edge; active app: 3×20 pt pill at the edge | 30 pt icon + name, badge on the right; active app row tinted, its windows listed below it as 22 pt rows (up to 6, then "n more") | As today's Dock: open or bring forward | As today's Dock icon menu |
| 4 | **Trash** | Fixed after the apps, pinned to the bottom group | "Trash" row | Opens Trash in Files | Open, Empty Trash… |
| 5 | **Workspaces** (only with 2+ workspaces) | Column of 6 pt dots, active one a 6×12 pill | "Desktops 1 2 3" pills | Click = switch; click active = overview | Overview, Add Desktop |
| 6 | **Tray** (StatusNotifierItems) | 2 per row, 26 pt cells, 15 pt glyphs; beyond 3 items the 4th slot is `•••` | One row of icons; `•••` beyond 6 | Item's own menu | Item's own menu |
| 7 | **Control Center** | Wi-Fi, sound, Bluetooth, battery as a 2×2 glyph grid inside **one** 52×58 button | One row: glyphs, then "Home-5G · 84 %", Control Center glyph | Opens Control Center | Wi-Fi glyph: network menu; sound: output menu; Bluetooth: devices; battery: battery menu (today's per-item menus, one step away) |
| 8 | **Clock + notifications** (bottom corner) | "14:32" 15 pt semibold tabular, "Thu 30" 11 pt muted; 7 pt accent dot when unread | "14:32" 22 pt, "Thursday 30 October", bell with dot | Opens Notification Center | Date & Time settings, Do Not Disturb toggle |

Scrolling over the Control Center button changes volume (today's behaviour on the sound item).
Battery percentage stays in the battery menu ("Show Percentage" item, roadmap decision) — in the
expanded bar it is shown because there is room.

Clock legibility: at 64 pt "14:32" in Inter 15 pt semibold is ~42 pt wide and fits on one line,
so the time is never stacked digit-pairs at the default width (stacked hours over minutes, as in
the design-language draft, is the fallback below 56 pt width). A 12-hour locale writes "2:32" with
"PM" as the second line's prefix ("PM · Thu 30").

### 3.2 Dimensions (points; drawn at output scale)

| Thing | Value |
|---|---|
| Collapsed width | `icon + 24`; default icon 40 → **64 pt**; Size slider icon 32–56 → width 56–80 |
| Expanded width | **248 pt** (fixed; app names to ~22 characters, then ellipsis) |
| Horizontal height | `icon + 16`; default **56 pt** |
| App cell | 48 pt along the bar (icon + 8); shrinks to icon 36 / cell 44 before overflow |
| Hairline separators | 1 pt, 32 pt long, 10 % label colour; only between groups |
| Mark cell | 52 pt; header 58 pt (+ 28 pt per menu row in expanded) |
| Status button | 52×58 pt (2×2 grid of 26 pt cells, 15 pt glyphs) |
| Clock block | 52 pt |
| Edge indicators | dot 4 pt; active pill 3×20 pt on the screen-edge side |
| Popovers (menus, Control Center, Notification Center) | 8 pt gap from the bar, 10–16 pt radius, aligned with the item that opened them |
| Hover label (collapsed) | today's Dock label (400 ms, glides between icons) — kept |
| Material | translucent with blur (`ext-background-effect-v1`), today's Dock/menu bar material; shadow on the content side, no border line |

**Attached or floating.** The mockups draw the bar attached to the edge, full height. The
design-language draft (`../datum/window-light.png`) draws a floating rounded rail with a
6 pt gap; both satisfy Fitts's law if, as today's Dock already does, the input region covers the
gap to the screen edge and the two corner items extend their hit area into the screen corners. The
floating form fits the draft's language; pick it if the draft is adopted, but keep the corner hit
areas.

### 3.3 App menus in the bar (the hard part)

Three approaches were drawn (`03-app-menu-alternatives.png`):

**A. Cascade from the app header (recommended as the primary path).** The header is the active
app. A click (or `Super+F2`) opens **one** menu beside the bar, top-aligned with the header:

1. The app row in bold (`Files ›`: About Files, Settings…, Services later, Hide Files `Super+H`,
   Hide Others, Show All, Quit Files `Super+Q`) — the macOS app menu, kept because it gives
   every app, even one that exports nothing, a complete menu.
2. Each top-level menu as a row with `›` (File, Edit, View, Go, Window, Help). Hover or Right
   opens its submenu to the side; a submenu safe-triangle (pointer moving diagonally toward the
   open submenu does not switch rows, as macOS and Amazon's mega-menu do) keeps it calm.
3. A quiet **footer tray** (`chassis`, hairline on top, 12 pt `ink3`): a magnifier, "Type to
   search" and the `Shift+Super+/` shortcut. It is a caption, not a row: it cannot be highlighted
   and is skipped by the arrow keys.

**Search in the cascade (decided 2026-10-30, after the owner's review of the first build).** The
first build put a "Search menus" field on top of the cascade. It cost a row on every open, pushed
the app's own menus down, looked like an empty row when nothing was typed, and was only useful
from the keyboard anyway. Now there is no field: typing any character while the cascade is open
turns its top into a compact **search line** (magnifier in `ink2`, the typed text in `ink`, the
1.5 pt accent caret, a hairline under it) and the rows become every enabled item of every menu
whose name contains the text, shown with its path ("Edit › Find") and shortcut, the first one
highlighted so Return runs it. Backspace to empty or Escape returns to the normal cascade; a
second Escape closes it. The footer disappears while searching. Screen readers hear the hint as
the menu's description and, while searching, a search input with the typed text; focus moves to
the first result as it would in any menu. The panel keeps its width while the list changes and
its height follows the results. Alternatives drawn in light and dark (in the review report, not
in the repo): **A** the field on top (the first build), **B** a "Search Menus… `Shift+Super+/`"
row at the bottom, **C** the footer tray (chosen). B reads cleanly but is a row that does what
typing already does, and clicking it still asks for the keyboard; C teaches both the gesture and
the global shortcut without adding a target. Goldens: `crates/shell/bar/tests/golden/cascade-*`.

**Header states.** At rest the header is a `well` group (radius 8) with the 24 pt icon and the
bold caption; hover adds 7 % `ink`, and from press until the cascade closes it holds 12 % `ink`,
so the header reads as the open menu's owner. The cascade appears on press, without delay, top-
aligned with the header 6 pt beside the bar. Clicking the bar never changes the active app: the
compositor keeps the key window while a shell surface (the bar or its menus) holds the keyboard,
so the header, its icon and the cascade's contents stay those of the app you were using.

Why: one stable target at a fixed place right under the corner, near every window's top-left
controls; works at any width and on every edge (on a right-edge bar it cascades leftwards, at
the bottom it opens upwards); uses the exported dbusmenu tree unchanged; costs one extra level
compared to the macOS strip, which the search removes for power users. It is NeXTSTEP's vertical
menu, made modern with search.

**B. Menu titles as rows in the expanded bar** — not an alternative but how A looks when the bar is
pinned open: the menu titles are listed under the app name, each row opening its submenu
directly (macOS's one-click reach to "Edit"). Collapsed bars cannot show them. The header row
still opens the same cascade (and `Super+F2` the same), so typing to search works identically in
both widths; the rows are the one-click shortcut into a single menu.

**C. Menu search (HUD), `Shift+Super+/`.** A centred card like the launcher, listing every enabled item
of the active app's menus by name, with its path ("Edit › Find") and shortcut; Return runs it,
`Super+Return` reveals it in the cascade (teaches where it lives, as macOS Help search points at
the item). It is the same search as the field in A. Rejected **as the only path** (Unity's HUD
lesson: invisible features are not discovered); adopted as the power layer. No compositor binding for `Shift+Super+/` was found in `crates/platform/compositor/src`, so it appears free; `Super+?` (`Shift+Super+/`) is the macOS Help search chord and
should alias it.

Rejected: **menus inside the Dock icon's right-click menu** (mixes "things about the app's
windows" with "commands inside the app", and makes the icon menu 40 rows long); **drawing a menu
strip into the top of the active window** (Unity's locally integrated menus — impossible for
client-side decorated apps and fights the title bar); **hover-to-reveal menus** (Unity 12.04's
discoverability failure).

**Horizontal bar.** At the top or bottom the header becomes today's menu bar: bold app name then
the menu titles inline (`04-desktop-bottom.png`). When the titles do not fit (narrow screens, many
apps), they collapse into the header button and A's cascade, from the right-most title inward, so
"Help" and "Window" go first.

**Apps that export no menu** (most GTK4, Electron and Qt apps until we read
`org_kde_kwin_appmenu`): the header still opens the app row (About is greyed when unknown, Hide,
Quit), the app's desktop-entry actions (New Window, …) and its window list, so the header is never
empty or dead. Their in-window menus are unaffected.

### 3.4 Expanded mode

Pinned, not hover. Hover-expanding sidebars move targets under the pointer and cover content
(the reason the Dock spec rejects magnification). The collapsed bar keeps today's hover labels.
Expanded is entered by dragging the bar's inner edge outward (snaps between 64 and 248), by the
bar's background menu ("Show Names"), or in Settings. It reserves its width; windows reflow.
Recommended on ≥ 2560 pt-wide screens (`07-wide-3440x1440.png`); the first-run assistant can
suggest it there, not force it.

### 3.5 Panels slide out of the bar

Launcher stays centred (it is a search, not a menu). Everything else opens from the item that
owns it, on the content side, with an 8 pt gap, aligned to the item and kept on screen:

- **App menu** — top-aligned with the header.
- **Control Center** — bottom-aligned with the status button (`06-small-1366x768.png`). Its first
  row gains the person: avatar and name, Lock, Power (menu: Sleep, Restart…, Shut Down…, Log
  Out…) and Settings, so session actions are reachable without the right-click system menu.
- **Notification Center** — full-height sheet beside the bar (`08-notification-center.png`),
  sliding out of the bar edge with the `standard` spring.
- **Banners** stay at the top of the far side (top right for a left bar), where they cover the
  least work; when put away they fly toward the clock and its unread dot appears.
- **Volume/brightness level card** — at the status button.

On a bottom bar they open upwards, right-aligned to the item, exactly as today's top-right panels
mirrored.

### 3.6 Behaviours

- **Overflow (many apps).** Cells shrink once (icon 40→36, cell 48→44); if the apps still do not
  fit, the last cell becomes "+N", opening a vertical list of the rest (same menu style, drag
  works from it). Kept apps are never pushed out before running-not-kept ones. Tray beyond three
  items gets "•••". The bottom group (Trash … clock) never scrolls or hides.
- **Small screens (1366×768).** 64 pt is 4.7 % of width — still cheaper than 80 pt of height.
  The header keeps its place; six to eight apps fit before "+N" (`06`).
- **Huge screens (3440×1440).** Expanded mode with window rows works as a vertical tab strip;
  2.0 % (collapsed) or 7.2 % (expanded) of width.
- **Multiple displays.** A bar on every display (roadmap A3 already wants the menu bar on every
  display and the Dock on the main one; one bar resolves that split). Each bar shows the same apps
  and status; the header follows the active app everywhere and is dimmed on displays that do not
  hold the key window, as macOS dims inactive menu bars. App icon rects for minimise go to the
  bar on the window's display.
- **Full screen.** The bar hides on that display; pushing the pointer against its edge for
  300 ms (longer than the 120 ms auto-hide dwell, so games and video do not reveal it by accident)
  slides it in over the app; `Super+F2`/`F3` and `Super+Space` reveal it instantly.
- **Auto-hide.** As today's Dock (120 ms dwell on a 2 pt strip, leaves 450 ms after the pointer,
  never while a menu or panel is open, slides in for drags). `Alt+Super+D` toggles (proposed in dock.md; no compositor binding found today). Hidden bars
  show a 2 pt sliver of the unread-notification accent at the corner when something is unread.
- **Drag and drop.** Unchanged from the Dock: files onto apps (accepting icon grows, others dim)
  and Trash, reorder by drag after 4 pt, drag out to remove, drag apps from the launcher or Files
  into the kept group. New: dragging a file onto the app header hands it to the active app.
- **Right click everywhere.** Every region has a menu (table in 3.1); the bar background menu
  has Show Names, Automatically Hide, Position ▸ (Left, Bottom, Right, Top), Bar Settings….
- **Keyboard.** `Super+F2`: app menu open with the first title selected (macOS Ctrl+F2 analogue,
  as today). `Super+F3`: the apps (today). `Shift+Super+/`: menu search. Inside the bar, Up/Down move
  within a region, Tab/Shift+Tab move between regions (launcher → header → apps → Trash →
  workspaces → tray → Control Center → clock), Return/Space activate, Menu or Shift+F10 the context
  menu, Escape returns the keyboard to the active app. Every item is an AccessKit node with role,
  name, state (running, badge count, unread) and actions; regions are groups with names.
- **Reduce Motion.** Slides become cross-fades; expand/collapse cross-fades between widths
  instead of animating the width; the launch pulse becomes a steady dot.
- **Light and dark** both designed (the mockups show light).

### 3.7 Other edges

- **Right**: mirror image; menus cascade leftwards; for right-to-left languages it becomes the default.
- **Bottom**: mark, header with inline menus, apps, Trash | workspaces, tray, Control Center,
  clock (two lines, time over date) at the right corner (`04`).
- **Top**: the same as bottom, at the top — effectively today's menu bar with the Dock inside it.
- **Separate menu bar** (setting): header, tray, Control Center and clock move to a 24 pt top bar
  (today's menu bar); the edge bar keeps launcher, apps, Trash, workspaces (today's Dock). This is
  today's layout, so it is also the migration safety net.

## 4. Settings (Desktop & Bar pane, all in `config.kdl`)

Only where people genuinely differ:

| Setting | Values | Why it is a setting |
|---|---|---|
| Position | Left (default), Bottom, Right, Top | Habit, screen shape and handedness differ; every precedent that removed this regretted it |
| Size | slider, icon 32–56 pt | Eyesight and screen density differ |
| Show names | off (default) / on (expanded) | Screen width differs by 2.5× between laptops and ultrawides |
| Automatically hide | off (default) / on | Small-screen users want every point |
| Menu bar | In the bar (default) / Separate, at the top | The one structural preference people hold strongly |

Not settings: tray layout, clock format (follows locale), status item selection (all present ones
are shown; apps' tray items are the app's choice), workspace indicator (shown when it means
something, i.e. 2+ workspaces).

```kdl
bar {
    edge "left"
    size 40
    names #false
    auto-hide #false
    menu-bar "in-bar"
    apps "org.bande.Files" "firefox" ...
}
```

## 5. Implementation plan

### 5.1 Architecture

- **One process, `bande-bar` (`crates/shell/bar`)**, replacing the `bande-dock` and
  `bande-menubar` binaries. Their logic is already mostly in libraries (`bande_dock::{DockState,
  arrange, …}` in `crates/shell/dock/src/lib.rs`; `bande_menubar` lib with `status`, `tray`,
  `dbusmenu`, `menus`, `session`, `devices`). Move the non-UI models that both orientations need
  into `crates/libs` (`bande-dock-model` for kept/running/arrange/badges/LauncherEntry,
  `bande-status` for network/sound/Bluetooth/battery/tray/session), keeping UI code in the bar.
- **One layer surface per output** (`zwlr_layer_shell_v1`, namespace `bar`), anchored to the
  edge, exclusive zone = width (or 0 with auto-hide), keyboard interactivity on demand (as the
  Dock layer already takes it). With "separate menu bar", a second surface per output, namespace
  `menubar`, exactly as today.
- **Popups** (`xdg_popup` from the layer surface) for app menu, icon menus, tray menus — the
  toolkit's popup path already used by both clients.
- **App menus**: unchanged transport — `com.canonical.dbusmenu` at `/org/bande/MenuBar` on the
  app's bus name (`bande-app-menu`). The menu search flattens the same layout tree
  (`GetLayout` with full recursion, `AboutToShow` on open). Later, add
  `org_kde_kwin_appmenu` in the compositor so Qt/KDE apps' dbusmenu is found per window.
- **Control Center / Notification Center** stay separate clients but read `bar { edge }` through
  `org.bande.Config` and an anchor rect from the bar to place themselves beside the item.

### 5.2 Protocol (`bande_shell_v1` v10)

| Need | Change |
|---|---|
| Keyboard targets | `focus_shell` keeps namespaces; the bar answers both "dock" (`Super+F3`) and "menubar" (`Super+F2`); add namespace "menu-search" for `Shift+Super+/` (compositor binding) |
| Panels beside items | new request `set_panel_anchor(surface, kind, x, y, w, h)` (kind: control_center, notifications) relayed to those clients, so Control Center and Notification Center open next to the item on any edge; or carry it in `toggle_control_center` as args |
| Full screen | new event `fullscreen(output, state)` so the bar switches to reveal-on-push; the compositor must stack the bar's overlay reveal strip above full-screen windows |
| Per-display active app | `active_app` gains the output of the key window (since 10) so inactive bars dim the header |
| Minimise targets | `set_icon_rect` unchanged, per surface (already per surface, so per output works) |
| Exclusive zones, per display | layer-shell already provides them; compositor needs multi-output (roadmap A3) |

### 5.3 Migration

- `config.kdl`: read `dock { apps size edge auto-hide }` when no `bar` block exists and write a
  `bar` block on first change; `DockApps`/`DockEdge`/… on `org.bande.Config` become aliases of
  `BarApps`/`BarEdge`/… for one release.
- Existing installs keep their layout: if a `dock` block exists, `menu-bar "separate"` is assumed
  (they get today's desktop); fresh installs get the combined left bar.
- The compositor starts `bande-bar` in place of both clients (`behaviour.md`, Shell clients);
  `bande-dock`/`bande-menubar` packages become transitional dummies depending on `bande-bar`.
- Goldens: today's dock and menu bar goldens survive as the "separate" mode goldens.

### 5.4 Order of work

1. **Host both in one process** with no visual change (dock surface + menu bar surface in
   `bande-bar`); goldens identical; compositor starts one client. Low risk, unlocks the rest.
2. **Vertical status region**: Control Center button (2×2), tray grid with overflow, clock block
   with unread dot, Trash and workspaces in the bottom group. Goldens light/dark at 1× and 1.5×.
3. **App header + cascade menu** (A) with submenu safe-triangle, keyboard (`Super+F2`), right-click
   menus; fall-back content for apps without dbusmenu.
4. **Menu search** in the cascade and `Shift+Super+/` card (needs compositor binding).
5. **Combined mode on by default** on the left; Settings pane (5 controls); config migration.
6. **Expanded mode** (names, inline menu rows, active app's window rows), drag-to-resize edge.
7. **Horizontal layouts** (bottom/top) with inline menu titles and their collapse order.
8. **Panels anchored to items** (Control Center, Notification Center) on every edge.
9. **Multi-display, full screen, auto-hide polish** (depends on roadmap A3).
10. **Third-party menus** via `org_kde_kwin_appmenu`.
11. Docs: update `docs/architecture/behaviour.md` (Dock, Menu bar paragraphs → Bar),
    `docs/design/features/dock.md` ("Clock, tray icons, workspace switcher in the Dock: never" is
    reversed by this proposal and needs the owner's decision), and an experience spec with the
    interaction table.

## 6. Trade-offs and risks

- **One more click to reach a menu** in the collapsed bar (header → Edit → item) versus macOS's
  strip. Mitigated by search (`Shift+Super+/`), shortcuts shown everywhere, and inline titles when
  expanded or horizontal.
- **The Dock spec said the Dock is "not a clock or tray"**. The combined bar deliberately
  reverses that; the separate-menu-bar setting preserves the old split.
- **Width on small laptops**: 64 pt on 1366 wide is 4.7 %; auto-hide is the escape, and the
  status/clock never hide.
- **Banners at the far corner from the clock** break the "everything from the bar" rule for one
  transient element; the alternative (banners beside the bar) covers the title bars of left-tiled
  windows.
- **Third-party app menus** will be thin until `org_kde_kwin_appmenu` is supported, and GTK4/
  Electron apps will never export; the header must never look empty (§3.3).
- **Multi-display bars on every screen** repeat the app list; some people will want the apps on
  the main display only. Wait for evidence before adding a setting.

## 7. Fit with the design-language research

`research/design-language.md` did not exist when this was written; its mockups in
`../datum/` did. They already draw a **left rail** combining apps, Trash, Wi-Fi, battery
and a stacked clock (`window-light.png`) — the same direction. Differences to reconcile: the
draft's rail is floating with circular icons and no app header or menus; this proposal adds the
header + cascade and the grouped Control Center button, and recommends keeping the floating
form only with edge-reaching hit areas (§3.2). The draft's grouped window controls pill at the
window's top left sits right of the header in the left bar, which keeps menu and window controls
close together.

## 8. Mockups

All in `../bar/` (SVG source + PNG):

| File | Shows |
|---|---|
| `01-desktop-left-collapsed` | 1920×1080, 64 pt bar: mark, Files header, apps with badges, Trash, workspaces, tray, Control Center button, clock with unread dot |
| `02-desktop-left-expanded` | 1920×1080, 248 pt pinned bar: search field, app menus as rows, apps with names and window rows, desktops, status row, full date |
| `03-app-menu-alternatives` | A cascade from the header (recommended), B rows in the expanded bar, C menu search `Shift+Super+/` |
| `04-desktop-bottom` | Horizontal 56 pt bar at the bottom with inline menu titles |
| `05-status-detail` | The bottom group collapsed (annotated) and expanded |
| `06-small-1366x768` | Small screen: smaller cells, "+5" overflow, tray "•••", Control Center open from the status button with user/lock/power row |
| `07-wide-3440x1440` | Ultrawide with the expanded bar and four tiled windows |
| `08-notification-center` | Notification Center sliding out beside the bar from the clock |
| `generate.py` | Generator for all of the above |
