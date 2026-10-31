# Desktop

Status: proposal, awaiting the owner's review.

## Purpose

The desktop is the surface behind every window. It shows the wallpaper and the files in the
Desktop folder, and it is the place where screenshots and drops land. In Bande it is a
calm, mostly empty place. Files on it behave exactly as they do in Files, and everything
on it can be reached with a right click or from the keyboard.

Current state (`docs/parity.md`, `NOTES/backlog.md`): the background is a flat colour.
There are no desktop icons and no desktop right-click menu. The lock screen and greeter
have no wallpaper. Screenshots already save to the XDG Desktop folder (M18), so the
desktop surface is needed before users can see them without opening Files.

## Reference survey

| Feature | macOS | GNOME / KDE / Windows | Note |
|---|---|---|---|
| Wallpaper sources | Dynamic (follows time of day), light and dark stills, aerials, pictures, solid colours, own photos, files or a folder [1] | | |
| Set from elsewhere | Right-click an image in Finder, then Set Desktop Picture. Safari: Use Image as Desktop Wallpaper [1] | | A right-click entry beats a settings trip |
| Desktop items | Documents, folders and aliases in the Desktop folder [3]. External disks optional (unverified) | GNOME: no desktop icons since 3.28 (unverified). KDE and Windows: icons | macOS and Windows users expect icons |
| Sort and arrange | View > Sort By (name, kind, date added, size, tags). Clean Up By is a one-time action. Sort By None keeps manual placement [3] | | |
| Stacks | Groups desktop files by kind, date or tags. New files join their stack. Click expands [2] | | Useful on cluttered desktops |
| View options | Icon size, grid spacing, text size, label position, item info [2][3] | | Mostly taste, rarely changed |
| Right-click menu | New Folder, Get Info, Change Wallpaper, Use Stacks, Sort By, Clean Up, Show View Options, Edit Widgets [2][3][4] (exact list unverified) | | |
| Show desktop | `Command-Mission Control` or `Fn-F11`, or spread thumb and three fingers. Windows move aside, and the same keys bring them back [6][7] | Windows: corner of the taskbar (unverified) | |
| Click wallpaper reveals desktop | Default "Always" since Sonoma [5][9] | | Users trigger it by accident and many turn it off [9][10] |
| Hide desktop items | "Show Items: On Desktop" toggle [5] | | People genuinely differ here |
| Widgets on desktop | Added from the right-click menu, Edit Widgets. They dim to monochrome when not in use [4] | KDE: desktop widgets. Windows: widgets board | |
| Open Desktop folder | `Shift-Command-D` [6] | | |
| Hot corners | Show Desktop is among the actions, with an optional modifier [11] | | |

## Proposed Bande feature set

### Now (essential)

**Wallpaper** — one image fills every display (scaled to cover, cropped at the centre). The
default is an Bande image with a light and a dark variant that follows the system
appearance, so light and dark mode both look intended [1]. Settings > Wallpaper offers the
bundled set, about six solid colours and Add Picture (a file or a whole folder).
Inputs: right click on the desktop > Change Wallpaper; right click on an image in Files >
Set as Wallpaper [1]; drop an image onto the Wallpaper pane.
Motion: a new wallpaper crossfades over about 400 ms, as does the light/dark switch, so
a change of appearance is never a hard cut.

**Wallpaper fit** — Fill (the default) or Fit. With Fit, the bars use the colour of the
image's edge (unverified as macOS behaviour; proposed). There are no Stretch, Tile or
Center options.

**Lock screen and greeter wallpaper** — the same image, blurred and darkened, so there is
nothing to configure separately (M17 and M24 gaps).

**Desktop items** — the files and folders in the XDG Desktop folder, shown as icons with
names on a grid. Removable and network volumes appear as well, so a plugged-in drive is
visible at once. Internal disks never appear. Items behave exactly as they do in Files,
using the same model, menus and Quick Look.
Inputs: left click selects; double click opens with the default app; `Return` and
`Super+Down` open (open question 1); hover shows the full name when it is truncated;
`Space` toggles Quick Look; `Super+I` shows Get Info; `Super+Backspace` moves to Trash;
`Super+C`, `Super+V` and `Super+D` (Duplicate, unverified as a desktop shortcut) work as in
Files; `Shift+Super+D` opens the Desktop folder in Files [6].
Motion: a new item fades and scales in at its slot. Items moved by Clean Up glide to their
new slots.

**Selection** — click selects one item. `Super+click` toggles an item and `Shift+click`
extends the selection. Dragging on empty desktop draws a rubber band that selects
everything it touches, live. `Super+A` selects all. Arrow keys move the selection, and
typing the start of a name selects it. Clicking empty desktop clears the selection and
leaves no window focused (`docs/architecture/behaviour.md`).
Inputs: left click, `Super`/`Shift` + click, left drag on empty space, keyboard.
Motion: the rubber band is drawn every frame. Selection highlights fade in over about
80 ms.

**Placement** — icons always sit on a grid; there is no Snap to Grid option. New items take
the first free slot from the top right, away from the left-side panel (macOS's top-right
origin, unverified). Dragged icons stay where they are dropped, and positions survive
restarts and display changes (stored per display configuration).
Inputs: left drag; `Clean Up` in the right-click menu packs icons to the grid in their
current order [3].

**Sort By** — None (the default: manual placement), Name, Kind, Date Added or Size. Any
choice but None keeps the desktop sorted as files arrive [3].
Inputs: right click > Sort By.

**Rename** — click the name of a selected item, or press `F2`, or choose Rename from the
menu. `Return` confirms and `Escape` cancels.

**Desktop right-click menu** — on empty desktop: New Folder, Paste (when the clipboard
holds files), Sort By (submenu), Clean Up, Change Wallpaper..., Show Desktop Folder in
Files. On an item: Open, Open With (submenu), Quick Look, Rename, Get Info, Copy,
Duplicate, Move to Trash, Show in Files, plus Set as Wallpaper for images and Eject for
volumes. With several items selected, the menu acts on all of them and its labels say so
("Move 3 Items to Trash").
Inputs: right click; `Shift+F10` or the Menu key on the selection (unverified as a
convention; proposed).

**Drag and drop onto the desktop** — the desktop is a normal drop target. Dragging files
from Files moves them if they are on the same volume and copies them otherwise. `Alt`
forces a copy and `Super` forces a move (Option and Command on macOS, unverified). Dropping
on a folder icon puts the files into the folder, and dropping on a volume copies them to
it. The pointer badge shows copy or move before release, so there are no surprises.
Inputs: left drag, with `Alt` or `Super` held while dropping.
Motion: dropped icons fly from the pointer to their slots. A cancelled drag springs back
to its source.

**Show desktop** — moves every window to the screen edges so the desktop shows, and the
same action brings them back [7]. Opening any window or switching apps also ends it. The
action only moves windows; it does not minimize them, so nothing is lost.
Inputs: the gesture of spreading thumb and three fingers [7]; `Ctrl+Super+D` (open
question 3); a hot corner if one is set (see windows.md). Clicking the wallpaper does not
trigger it (open question 2).
Motion: windows slide off to the nearest edge with the shared spring, leaving a sliver
visible, and slide back on the second invocation. Mid-gesture reversal is supported [8].

**Show items on desktop** — one setting, on by default, in Settings > Desktop & Dock [5].
When it is off the desktop shows only the wallpaper, and files remain in Files.

### Later (power features worth having)

**Stacks** — groups desktop files into piles by Kind (the default) or Date Added. Clicking a
stack fans it open in place, and clicking its header closes it. New files land in their
stack [2]. This is the answer to a cluttered desktop, behind the right-click menu.
Inputs: right click > Use Stacks; click; two-finger horizontal scroll over a stack flips
through its items [2].
Motion: the stack fans out and folds back along the grid.

**Wallpaper per display** — a display picker in the Wallpaper pane. The default stays "same
on all displays".

**Time-of-day wallpaper** — bundled images with several states that shift from dawn to
night [1], computed from the clock without location services. This is an extension of the
light/dark pair.

**Rotate pictures from a folder** — a new picture every hour or day, or at login [1].

**Drop content from apps** — text or an image dragged from an app onto the desktop becomes
a file (a `.txt`, or an image in the dragged format). The screenshot thumbnail already
works this way (`docs/design/catalogue.md` §6).

**Spring-loaded folders** — holding a drag over a folder icon opens it in Files so the drop
can go deeper (macOS Finder behaviour, unverified).

**Icon size** — one slider (small, medium, large). It is the one view option where people
differ, mostly for eyesight.

### Never (left out)

- **Trash on the desktop** — Trash lives in the panel, one place only (`docs/parity.md`).
- **Drag a volume to Trash to eject** — the gesture means delete everywhere else. Eject is
  in the right-click menu and the Files sidebar.
- **Home, Computer and Network icons** — the panel and Files already reach these, so the
  icons would be a second way to the same thing.
- **Grid spacing, label position and text size options** — rarely changed, and a source of
  settings clutter [2][3]. Bande picks good values instead.
- **Widgets on the desktop** — they are hidden behind windows most of the time. Glanceable
  information lives in the menu bar and Notification Center (open question 4) [4].
- **Animated or aerial wallpapers** — they cost power, distract and need a large download
  [1].
- **Click wallpaper to reveal desktop** — it is triggered by accident during ordinary
  clicks [9][10]. One deliberate gesture and one shortcut are enough.
- **Per-workspace wallpapers** — a setting for few people that makes workspaces harder to
  explain. Wallpaper is per display only.
- **Tiled, stretched and centred fits** — Fill and Fit cover real images, and the rest are
  legacy options.

## Open questions for the owner

1. **What does `Return` do on a selected icon?** macOS renames on Return (unverified in
   these sources). Recommendation: `Return` opens and `F2` or a click on the name renames.
   Opening is what users expect from Linux and Windows, and the desktop and Files must
   agree, so this decision also binds Files.
2. **Should clicking the wallpaper reveal the desktop?** macOS turned it on by default in
   Sonoma [5], and users disable it [9][10]. Recommendation: no, and no setting either.
3. **Show desktop shortcut.** macOS uses `Command-F11` or the Mission Control key with
   Command [6][7]. F11 is full screen in browsers on Linux. Recommendation:
   `Ctrl+Super+D`, which fits the `Ctrl+Super` window-chord family proposed in windows.md.
4. **Desktop widgets: never, or later?** Recommendation: never for 1.0. A clock, weather
   and calendar belong in the menu bar and Notification Center, so there is one place to
   glance. Revisit if Notification Center gains widgets.
5. **Which process draws the desktop?** Recommendation: a small shell client,
   `bande-desktop`, on the background layer. The compositor starts and restarts it as it
   does the panel. It reuses the file model, menus and Quick Look from `bande-files` as a
   library, so a Files crash does not take the desktop down, and the desktop never needs a
   Files window open.

## Sources

1. Apple, Change your wallpaper on Mac: https://support.apple.com/guide/mac-help/change-your-wallpaper-mchlp3013/mac
2. Apple, Use stacks on the Mac desktop: https://support.apple.com/guide/mac-help/use-stacks-on-the-mac-desktop-mh35846/mac
3. Apple, Ways to organize files on your Mac desktop: https://support.apple.com/guide/mac-help/mh35951/mac
4. Apple, Add and customize widgets on Mac: https://support.apple.com/guide/mac-help/add-and-customize-widgets-mchl52be5da5/mac
5. Apple, Change Desktop & Dock settings on Mac: https://support.apple.com/guide/mac-help/mchlp1119/mac
6. Apple, Mac keyboard shortcuts: https://support.apple.com/en-us/102650
7. Apple, Use Mission Control on Mac (show desktop): https://support.apple.com/guide/mac-help/mh35798/mac
8. Apple HIG, Motion: https://developer.apple.com/design/human-interface-guidelines/motion
9. MacMost, How to turn off the reveal desktop feature: https://macmost.com/how-to-turn-off-the-reveal-desktop-feature-on-your-mac.html
10. Popular Science, Stop macOS Sonoma from hiding your windows: https://www.popsci.com/diy/macos-sonoma-hiding-windows/
11. Apple, Use hot corners on Mac: https://support.apple.com/guide/mac-help/use-hot-corners-mchlp3000/mac
