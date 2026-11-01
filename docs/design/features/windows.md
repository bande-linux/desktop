# Windows

Status: proposal, awaiting the owner's review.

## Purpose

Windows are where all work happens. Bande gives every window a single, consistent set of
gestures to move, resize, arrange, hide and find it again. Each gesture is animated and can
be interrupted. The basics need no explanation and no setting. Power features (tiling,
workspaces, the overview) are one modifier, one right-click or one shortcut away.

Current state (`docs/parity.md`, `NOTES/backlog.md` M15, M26): the compositor has xdg
move and resize grabs, zoom into the area outside exclusive zones, full screen
(`Ctrl+Super+F`), minimize, `Super+W`, `Super+Q` and `Super+Tab` without an overlay. None of
these is animated. Title bars are client-side for our toolkit and for GTK windows with a
header bar, and drawn by the compositor in the same style for apps that ask for server-side
decorations (Qt, foot, SDL, framed Electron windows, GTK windows without a header bar). There is no snapping, no
overview and no workspaces.

## Reference survey

| Feature | macOS | GNOME / KDE / Windows | Note |
|---|---|---|---|
| Window controls | Close, minimize and zoom at the top left. They are coloured on the key window and grey on other windows [10] | GNOME: close only by default (unverified) | Bande keeps all three |
| Green button | Click enters full screen. Hover shows Move & Resize, Fill & Arrange and Full Screen [3][4] | Windows: hovering maximize shows Snap layouts [18] | The hover menu takes a second or two to appear, which reviewers criticise [4] |
| Double-click title bar | Zooms (Fill) or minimizes, chosen in Desktop & Dock [2][9] | GNOME: maximize [14] | One fixed action is enough |
| Move | Drag the title bar. A window snaps into alignment next to another window [2] | GNOME: drag the title bar | |
| Resize | Drag any edge. Double-clicking an edge expands that side [2] | Same everywhere | Edge snapping to neighbours stops at alignment [2] |
| Minimize | `Command-M`. `Option-Command-M` minimizes all of the app's windows [1]. Genie or Scale effect, optionally into the app icon [9] | GNOME: no minimize button by default (unverified) | |
| Hide app | `Command-H`, `Option-Command-H` hides the others [1] | Not in GNOME or KDE | A whole-app concept that minimize does not cover |
| Full screen | `Control-Command-F`. The window gets its own Space. The menu bar and Dock reveal on hover [1][5] | GNOME: full screen in place, without a new workspace (unverified) | |
| Tiling, drag | Edges give halves, corners give quarters, the menu bar gives fill. Option-drag tiles anywhere. Optional margins. Dragging away restores the size [4][9] | GNOME: halves, top maximizes [13][14]. Windows: corners give quarters, the top edge shows a layout bar [18]. KDE: custom tile layouts, Shift-drag into tiles, shared edges resize together [19] | Reviewers report accidental tiles that are hard to undo [4] |
| Tiling, keyboard | `Fn-Control-Arrow` for halves, `Fn-Control-F` fill, `Fn-Control-R` return to previous size [3] | GNOME: `Super+Left/Right/Up` [13][14]. Windows: `Win+Arrow`, `Win+Z` [18] | The `Fn` key is missing from most PC keyboards [4] |
| Fill the rest | Fill & Arrange fills the other regions with recently used windows [3][4] | Windows: Snap Assist offers the other windows as thumbnails [18] | Windows' pick-a-window approach is more predictable than macOS's guess [4] |
| App switcher | `Command-Tab` overlay of apps, arrow keys, `Esc` cancels [1][2] | GNOME: `Super+Tab` grouped by app; Down shows an app's windows [15] | macOS does not restore minimized windows from the switcher without an Option trick [21] |
| Same-app windows | ``Command-` `` and ``Shift-Command-` `` [1] | GNOME: ``Super+` `` [15] | |
| Overview | Mission Control: all windows in one layer and a Spaces bar. `Control-Up`, three-finger swipe up [6] | GNOME: Activities overview with search and workspaces [17] | |
| App windows | App Exposé, `Control-Down` [1][6] | | |
| Workspaces | Spaces, added by hand in Mission Control, up to 16. Drag a window to the screen edge to move it to the next Space. Apps can be assigned to Spaces [7] | GNOME: dynamic, with always one empty workspace [16] | GNOME's dynamic model needs no add and remove step |
| Stage Manager | Recent apps as a strip at the side, groups of windows [8] | none | Off by default on macOS [9] |
| Window menu | Minimize, Zoom, Full Screen, Bring All to Front (Option: Arrange in Front), list of windows [11] | KDE and GNOME: a title-bar context menu (unverified) | macOS has no title-bar context menu (unverified) |
| Multiple displays | With "Displays have separate Spaces", each display has its own menu bar and Spaces. Without it, a window can span displays [9][20] | | |
| Motion | "Let people cancel motion"; "Make motion optional" [12] | | |

## Proposed Bande feature set

### Now (essential)

**Title bar** — drawn by the app through the toolkit. It holds the window controls at the
top left, the title in the centre and optionally a toolbar. The whole bar is a drag handle
except where it has controls.
Inputs: left drag moves the window; double click zooms; right click opens the window menu
(below); hover over the controls shows their glyphs.

**Window controls** — close, minimize and zoom, in macOS order, in Bande's own style. They
are coloured on the key window and grey on others [10]. Hovering over the group shows
all three glyphs.
Inputs: left click acts. Alt+click on minimize minimizes all of the app's windows. Hover
over zoom for about 0.5 s, or right click it, opens the layout menu (tiles and full screen).
Click zoom with Alt held to enter full screen.
Motion: glyphs fade in over about 100 ms.

**Window menu** — the same menu from right click on the title bar or right click on zoom,
and from the app's Window menu in the menu bar. Items: Minimize, Zoom, Full Screen, Tile
(Left, Right, Top Left, Top Right, Bottom Left, Bottom Right), Return to Previous Size, Move
to Workspace (a list), Move to Display (a list), Close. This is the one place every window
action can be found, so there are no dead ends.
Inputs: right click; `Alt+Space` opens it from the keyboard (KDE and GNOME convention,
unverified; Alt is free here because Super is Command).

**Move** — drag the title bar. Windows never go under the menu bar (M15 precedent). Near a
screen edge or a neighbouring window's edge, the window snaps to it with about 8 px
resistance [2].
Inputs: left drag. `Ctrl+Super`+drag anywhere in a window moves it, for apps whose title
bars are hard to grab. Ctrl+Super is already the compositor's window chord (`Ctrl+Super+F`),
and plain Super+drag stays with apps as Command-drag.

**Resize** — drag any edge or corner through a grab band that reaches 8 pt outside the
window and 3 pt inside its rounded outline; corners reach 24 pt along each edge. The cursor shows the direction. Edges snap to the screen and to neighbours.
Inputs: left drag. Alt+drag resizes symmetrically about the centre (macOS Option behaviour,
unverified). Double click on an edge extends that side to the screen edge [2].

**Zoom** — toggles between the user's size and filling the visible area (outside the menu
bar and the panel). This already exists. Bande's green button zooms, not full screen,
because filling the screen is the common need (see open question 1).
Inputs: click zoom; double click the title bar; `Ctrl+Super+Up`; drag the window to the top
edge, under the menu bar [4].
Motion: the frame interpolates position and size with a critically damped spring
(about 250 ms). The client's buffer is scaled until it commits the new size.

**Tiling** — halves and quarters of the visible area, with no gaps by default. Dragging a
tiled window away, or Return to Previous Size, restores its earlier size [4]. Two windows
tiled side by side share their edge: dragging it resizes both (KDE [19]).
Inputs: drag to the left or right edge of the visible area for halves (a taskbar on that
side counts as edge) and to a corner for quarters. A translucent outline previews the
target as soon as the pointer reaches the edge of the free area. Between two displays the
pointer must rest 250 ms, so moving a window across does not tile it.
Keyboard: `Super+Left/Right` for halves. Pressing `Super+Up/Down` on a half then
gives its top or bottom quarter. `Super+Down` on an untiled or zoomed window returns
it to its previous size. Every tile is also in the window menu.
Motion: the outline grows from the pointer in about 0.12 s, and the window springs into the
outline in about 0.18 s when released.

**Minimize** — sends the window into its app's icon in the panel. There are no separate
window tiles in the panel (macOS's "Minimize windows into application icon" [9], made the
only behaviour). Clicking the app icon restores the latest minimized window when the app
has no visible window (M15 precedent). The icon's right-click menu lists minimized windows.
Inputs: yellow button; `Super+M`; `Alt+Super+M` for all of the app's windows [1].
Motion: the window scales and fades towards the icon (macOS's Scale effect [9]), about
250 ms. Restore plays it in reverse from wherever it is.

**Hide app** — hides all of an app's windows. The app stays running and its icon keeps its
dot. Activating the app (panel, switcher, launcher) shows them again.
Inputs: `Super+H`; `Alt+Super+H` hides every other app [1]; `H` in the app switcher.
Motion: the windows fade out together, about 150 ms.

**Full screen** — the window covers the display. The menu bar slides in when the pointer
touches the top edge, and the panel when it touches the panel's edge [5]. In the first
version the window stays in place and does not get a workspace of its own (open
question 2).
Inputs: `Ctrl+Super+F` (exists); Alt+click zoom; layout menu; Escape does not leave full
screen, because apps such as video players use it themselves.
Motion: the frame springs to the display bounds while the menu bar and panel slide out.

**Focus and raising** — as already decided in `docs/architecture/behaviour.md`: click to focus and raise,
no focus stealing, and close or hide focuses the app's next window. To this Bande adds a
dimmed inactive state. Inactive windows have grey controls and a lighter shadow, so the key
window reads as nearest [10].
Inputs: left click anywhere in the window. A click that focuses a window also reaches the
app (macOS's click-through behaviour for title bars and controls is app-defined,
unverified).

**App switcher** — `Super+Tab` shows a centred row of app icons in recent-use order. A
quick tap with release under about 150 ms switches without showing the overlay, so it does
not flicker. Choosing an app whose windows are all minimized restores the latest one,
which fixes the macOS oddity in [21].
Inputs: `Tab` and `Shift+Tab` or Left and Right cycle. Down shows the selected app's
windows as thumbnails to choose from (GNOME [15]). `Q` quits and `H` hides the selected
app (macOS behaviour, unverified). `Escape` cancels [2]. Hover selects and click chooses.
Release `Super` to switch.
Motion: the overlay fades and scales in from 0.96, about 120 ms. The selection ring slides
between icons.

**Window cycling** — ``Super+` `` and ``Shift+Super+` `` step through the focused app's
windows with no overlay [1].

**Animations everywhere** — open (scale from 0.95 and fade, about 200 ms), close (the
reverse, about 150 ms, after the client has unmapped, using the last buffer), minimize,
zoom, tile, full screen and switcher. All use one spring model, which `docs/design/catalogue.md` §7
already specifies for gestures. Each animation retargets from its current value when
interrupted, so a second `Super+M` mid-flight reverses smoothly [12]. Reduce Motion in
Settings replaces movement with short crossfades [12].

### Later (power features worth having)

**Overview** — all windows of the current workspace scaled into a grid without overlap,
with a workspace strip at the top. It is Mission Control and Exposé in one view. It does
not search, because the launcher already searches; there is one way to search.
Inputs: three-finger swipe up (tracks 1:1, `docs/design/catalogue.md` §7); the keyboard Mission Control
key (`F3`, unverified on PC layouts); `Alt+Super+Up` (open question 3); dragging a window
to the top edge and holding it there (macOS option [9], off by default). Hovering over a
window shows its title. Click picks a window. Drag moves a window to a workspace or display.
Right click opens the window menu. Arrow keys and Return pick. Escape or a click on empty
space closes the overview.
Motion: windows fly from their real positions to the grid with the same spring, and back
when the overview closes, including mid-gesture.

**App windows** — the overview filtered to one app (App Exposé [6]).
Inputs: `Alt+Super+Down`; "Show All Windows" in the panel icon's right-click menu; Down
in the app switcher.

**Workspaces** — a horizontal row, created as needed. There is always one empty workspace
at the end, and a workspace that empties is removed (GNOME [16]). There is nothing to add
or delete by hand.
Inputs: three- or four-finger horizontal swipe (1:1, spring on release); `Alt+Super+Left/Right`;
`Alt+Super+1..9`. Adding `Shift` carries the focused window along. Drag in the overview.
Window menu, Move to Workspace. Activating an app whose only windows are on another
workspace switches to that workspace (macOS default [7]).

**Fill the other half** — after a window is tiled to a half or quarter, the free area shows
the other windows as thumbnails. Click one to fill it, or press Escape or click elsewhere to
dismiss (Windows Snap Assist [18]). It is predictable because the user picks, rather than
the system guessing as Fill & Arrange does [4].

**Multiple displays** — each display has its own workspaces and a menu bar showing the
active app (macOS's separate-Spaces model [9][20]). The panel stays on the primary display.
A window that crosses displays belongs to the display holding most of it, and windows
cannot span displays. When a display is disconnected its windows move to the remaining
display. When it is reconnected they return to it (unverified as macOS behaviour; proposed
as Bande's).
Inputs: drag across; window menu, Move to Display; `Ctrl+Alt+Super+Left/Right` moves the
focused window to the neighbouring display.

**Hot corners** — off by default. Each corner can be set in Settings to Overview, Show
Desktop, Lock Screen or Notification Center, with an optional modifier [9]. This is a
power feature behind a setting.

**Keep on top** — a window menu toggle for a picture-in-picture-style window (KDE and GNOME
offer this, unverified).

### Never (left out)

- **Stage Manager** — a second window model that duplicates the switcher, workspaces and
  tiling. Two ways to do one thing breaks "one obvious way" [8].
- **Genie effect** — one minimize motion is enough, and Scale reads better at Bande's
  speeds [9].
- **Full-screen Split View spaces** — tiling two windows inside a full-screen workspace
  duplicates plain tiling. macOS itself now hides it in a submenu [4].
- **Fill & Arrange guessing** — the system picking windows for the user is unpredictable
  [4]. "Fill the other half" covers the need.
- **Drag to the screen edge to change workspace** — it conflicts with edge tiling, and the
  same edge cannot mean two things [7].
- **Manual "add Space" and app-to-Space assignment** — dynamic workspaces remove the need
  [7][16].
- **Focus follows mouse, window shading, window opacity menus** — settings for the few that
  complicate the model for everyone.
- **Tiling margins setting** — gaps are a taste that is not worth a setting. Bande picks
  zero, and windows' own shadows separate them [4].
- **Custom tile-layout editor** — KDE's editor [19] is real power but a second
  configuration surface. Halves, quarters and the shared-edge resize cover most needs.
  Revisit for ultrawide displays.
- **Automatic tiling as a mode** — a different paradigm from floating windows. One window
  model.

## Open questions for the owner

1. **Green button: zoom or full screen?** macOS makes click full screen [5]. Recommendation:
   click zooms (fills the visible area), and Alt+click or the layout menu enters full
   screen. Filling is the frequent need, and full screen hides the menu bar and panel.
2. **Full screen: a workspace of its own, or in place?** macOS gives it a Space [5].
   Recommendation: in place for now, because it needs no workspaces and introduces no new
   concept. Revisit when workspaces land, since the swipe between a full-screen app and
   the desktop is a real laptop strength.
3. **Shortcut family.** `docs/design/catalogue.md` §8 proposes `Super+Left/Right` for tiling and §7
   proposes `Super+Ctrl+Left/Right` for workspaces. `Super+Left/Right` is line start and end
   in text (the keyboard model in `docs/architecture/overview.md`) and `Super+Up` is "enclosing folder" in Files (M22). Recommendation:
   `Ctrl+Super+Arrows` places the focused window (as `Ctrl+Super+F` already does full
   screen), and `Alt+Super+Arrows` moves between views (overview, app windows,
   workspaces). Amend 6.7 and 6.8 to match.
4. **Workspaces per display or spanning all displays?** Recommendation: per display, which is
   macOS's default [9]. It keeps a menu bar on each display and lets one display change
   workspace while another stays put. The cost is one more concept for multi-display users.
5. **Should Hide (`Super+H`) exist alongside Minimize?** They overlap. Recommendation: keep
   both. Hide is per app and leaves no trace in the panel. Minimize is per window. macOS
   users rely on `Command-H` [1], and the cost is one shortcut and one switcher key.

## Sources

1. Apple, Mac keyboard shortcuts: https://support.apple.com/en-us/102650
2. Apple, Work with app windows on Mac: https://support.apple.com/guide/mac-help/mchlp2469/mac
3. Apple, Tile app windows on Mac: https://support.apple.com/guide/mac-help/tile-mac-windows-mchl9674d0b0/mac
4. TidBITS, How to Tame Sequoia's Window Tiling: https://tidbits.com/2024/11/08/how-to-tame-sequoias-window-tiling/
5. Apple, Use apps in full screen on Mac: https://support.apple.com/guide/mac-help/mchl9c21d2be/mac
6. Apple, Use Mission Control on Mac: https://support.apple.com/guide/mac-help/mh35798/mac
7. Apple, Work in multiple spaces on Mac: https://support.apple.com/guide/mac-help/mh14112/mac
8. Apple, Use Stage Manager on Mac: https://support.apple.com/guide/mac-help/mchl534ba392/mac
9. Apple, Change Desktop & Dock settings on Mac: https://support.apple.com/guide/mac-help/mchlp1119/mac
10. Apple HIG, Windows: https://developer.apple.com/design/human-interface-guidelines/windows
11. Apple HIG, The menu bar (Window menu): https://developer.apple.com/design/human-interface-guidelines/the-menu-bar
12. Apple HIG, Motion: https://developer.apple.com/design/human-interface-guidelines/motion
13. GNOME Help, Tile windows: https://help.gnome.org/users/gnome-help/stable/shell-windows-tiled.html.en
14. GNOME Help, Maximize and unmaximize a window: https://help.gnome.org/users/gnome-help/stable/shell-windows-maximize.html.en
15. GNOME Help, Switch between windows: https://help.gnome.org/users/gnome-help/stable/shell-windows-switching.html.en
16. GNOME Help, What is a workspace: https://help.gnome.org/users/gnome-help/stable/shell-workspaces.html.en
17. GNOME Help, Visual overview of GNOME: https://help.gnome.org/users/gnome-help/stable/shell-introduction.html.en
18. Microsoft, Snap your windows: https://support.microsoft.com/en-us/windows/snap-your-windows-885a9b1e-a983-a3b1-16cd-c531795e6241
19. Marco Martin (KDE), KWin and tiling: https://notmart.org/blog/2022/10/kwin-and-tiling/
20. MacMost, Macs with two displays: separate Spaces: https://macmost.com/macs-with-two-displays-choose-windows-across-displays-or-separate-spaces.html
21. Macworld, Command-Tab and minimized windows: https://www.macworld.com/article/206271/commandtabminimizedwindows.html
