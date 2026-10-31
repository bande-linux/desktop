# Window management in motion

Research date: 2026-10-29. This file covers how window operations move and connect.
`docs/design/features/windows.md` already proposes the feature set; this file adds behaviour
details, timings and the component work. "Unverified" marks what we could not check.

## What macOS does

### Tiling (Sequoia and later)

- **Settings.** Desktop & Dock > Windows has four switches:
  - "Drag windows to screen edges to tile".
  - "Drag windows to menu bar to fill screen".
  - "Hold Option key while dragging windows to tile".
  - "Tiled windows have margins".
  - Source: <https://tidbits.com/2024/11/08/how-to-tame-sequoias-window-tiling/>
- **Behaviour while dragging.**
  - Dragging to an edge shows an outline; releasing tiles the window. The menu bar fills.
  - With Option held, tiling works even with the edge switches off.
  - Dragging a tiled window away "reverts to its pre-tiled size" (TidBITS).
  - Whether corners tile without Option is disputed between sources (unverified).
- **Timing.** Apple publishes no delay before the outline appears. Users report macOS is
  "less eager" than Windows, which suggests a short dwell (anecdotal, TidBITS comments).
- **Menu and shortcuts.** Green-button hover, Window > Move & Resize and Full Screen Tile list
  the layouts. <https://support.apple.com/guide/mac-help/tile-app-windows-mchl9674d0b0/mac>
  - Fn-Ctrl-F: fill.
  - Fn-Ctrl-C: centre.
  - Fn-Ctrl-Arrows: halves.
  - Fn-Ctrl-R: return to previous size.
  - Fn-Ctrl-Shift-Arrows: two-window arrangements.
  - Fn-Ctrl-Option-Shift-Arrows: half and quarters.
  - Corners have no shortcut.
- **Margins.** The margins switch adds a gap of a few px (value unpublished).
  <https://appleinsider.com/inside/macos-sequoia/tips/how-to-turn-off-the-gaps-in-macos-sequoias-window-tiling>

### Zoom, full screen, minimise, hide

- **Zoom and resize.**
  - Animated at `NSWindowResizeTime` 0.20 s per 150 px (verified,
    <https://developer.apple.com/documentation/appkit/nswindow/animationresizetime(_:)>).
  - Longer moves take longer, but sub-linearly in practice (unverified).
  - `defaults write -g NSWindowResizeTime 0.001` is the popular "make it instant" tweak, which
    tells us power users find it too slow.
- **Full screen.**
  - The window moves to its own Space with a zoom while the desktop slides away.
  - The menu bar and Dock reveal on hover.
  - Durations are unpublished; about 0.5 to 0.7 s is often cited (unverified). It is the
    slowest common transition on macOS and the one most complained about.
- **Minimise.**
  - Genie or Scale into the Dock or the app icon.
  - The window shrinks toward the exact Dock slot it will occupy; restore plays it in reverse
    from that slot.
  - Durations are unpublished.
- **Hide (Command-H).** All windows of the app disappear together with a short fade, with no
  spatial motion (unverified).
- **Window behaviours.** `NSWindow.AnimationBehavior` (document window, utility window, alert
  panel, none) chooses the order-front and order-out animation per window type, so alerts pop
  differently from documents.
  <https://developer.apple.com/documentation/appkit/nswindow/animationbehavior-swift.enum>
  (cases from memory, unverified).

### App switcher and window cycling

These are well known but unverified on fetched pages.

- **Command-Tab:**
  - Shows icons in recency order. A quick press and release switches without visibly
    flashing the overlay.
  - Holding Command lets Tab and backtick step forward and back.
  - Q quits and H hides the selected app.
  - Up or Down opens App Exposé for it.
  - The pointer can hover and click icons.
  - Dropping a dragged file onto an icon in the switcher opens it there.
- **Command-backtick** cycles the app's windows with no overlay.

### Mission Control, App Exposé, Stage Manager

- **Mission Control:**
  - Shows all windows in one layer with the Spaces bar along the top. "Group windows by
    application" is an option.
  - Entered with Ctrl-Up, the Mission Control key, a 3/4-finger swipe up, or by dragging a
    window to the top edge (option).
  - Dragging a window onto a full-screen thumbnail creates Split View.
  - Source: <https://support.apple.com/guide/mac-help/view-open-windows-spaces-mission-control-mh35798/mac>
- **Layout behaviour** (observed, unverified):
  - Windows keep their relative positions and z-order hints. They scale down into a
    non-overlapping arrangement near where they were, so the eye can follow each window.
  - The Spaces bar shows names, and expands to thumbnails when the pointer touches it.
  - The gesture drives the transition progressively, and lifting early returns.
- **Stage Manager:** the current group is in the centre, with recent groups as tilted
  thumbnails in a strip on the left. Click swaps; dragging a thumbnail into the stage adds it
  to the group. It is off by default (unverified details). Worth understanding for one idea
  only: **a window is never lost, its thumbnail stays where you saw it last.**

## What others do better

- **Windows 11 Snap Layouts** (<https://support.microsoft.com/en-us/windows/snap-your-windows-885a9b1e-a983-a3b1-16cd-c531795e6241>):
  - Hovering maximise shows a grid of layouts to click. Dragging toward the top of the screen
    drops down a Snap bar with the same layouts.
  - Snap Assist then offers the other windows to fill the rest.
  - Snap Groups come back together from the taskbar.
  - The best part is the drag-to-top bar: it makes layouts reachable mid-drag without keyboard
    or precision.
- **KWin:**
  - Quick tiling shares the edge between two tiled windows, and dragging that edge resizes
    both.
  - Effects are short: Scale 200 ms, Glide 160 ms, Magic Lamp 250 ms, Squash 250 ms. One
    global speed factor scales all of them.
    <https://raw.githubusercontent.com/KDE/kwin/master/src/plugins/scale/package/contents/code/main.js>
- **niri:**
  - Every window movement, resize, view move and overview open is a critically damped spring
    (ζ 1.0, k 800, about 0.22 s perceptual). Interruption retargets with velocity preserved.
    <https://github.com/YaLTeR/niri/wiki/Configuration:-Animations>
  - The overview zooms out the real workspace instead of rebuilding a separate scene, so
    windows never jump.
- **GNOME:**
  - Window open and close take 150 ms.
  - Tile preview and size change take 250 ms ease-out-quad.
  - Minimise takes 400 ms.
  - <https://gitlab.gnome.org/GNOME/gnome-shell/-/raw/main/js/ui/windowManager.js>

## The integration lessons

1. **Every window transition has a source and a destination on screen**:
   - Minimise goes to the exact icon.
   - Restore comes from it.
   - Mission Control windows fly from their real positions.
   - Quick Look comes from the icon.
   - Full screen grows from the window.

   Nothing appears from nowhere.
2. **One window, one identity.** The same window texture is used in place, in Mission Control,
   in the switcher and in Exposé. It is scaled, never re-rendered as a different-looking
   thumbnail.
3. **Configure early, animate the buffer.** macOS animates the frame while the app redraws at
   the new size (`animationResizeTime`). On Wayland the compositor must animate the old buffer
   scaled toward the target and cross-fade to the new buffer when the client commits the new
   size.

## For Bande

All of this is `crates/platform/compositor` unless noted.

1. **Window animation core** (now).
   - Each window gets one `WindowMotion` holding springs for x, y, w, h, scale and opacity,
     sampled at the predicted presentation time.
   - Every operation (open, close, zoom, tile, minimise, restore, workspace move, overview)
     just sets new targets. Interruption therefore works everywhere for free.
   - Close renders the last buffer until the animation ends. Input goes to the next window
     immediately.
2. **Resize transitions** (now).
   - Send `configure` with the target size at the start, and render the current buffer scaled
     to the spring's current rectangle.
   - When the client acks and commits, cross-fade over 100 ms from the scaled old buffer to the
     new buffer stretched to the current rectangle.
   - Clients that do not commit within 150 ms keep the scaled buffer until they do. niri and
     KWin do the same; rounded-corner clipping hides the stretch.
3. **Tiling preview** (now; matches windows.md).
   - The outline appears after the pointer rests 150 ms within 2 px of the edge zone. The zone
     is 1 px at the screen edge plus a 24 px band, which is crossed only at low speed.
   - It grows from the pointer with `quick`.
   - On release the window springs into it with `standard`. Dragging a tiled window away
     restores its earlier size while staying under the pointer, keeping the grab point
     proportional.
   - Add a Snap-bar equivalent later: dragging to the top edge and holding 300 ms drops a
     small layout bar from the menu bar (halves, quarters, fill), driven by `quick`.
4. **Minimise into the icon** (now, compositor plus `bande-dock`).
   - The Dock reports each app icon's rectangle to the compositor via `bande_shell_v1`
     (`set_icon_rect(app_id, rect)`). `zwlr_foreign_toplevel_handle.set_rectangle` exists for
     the same purpose in wlr; `ext-foreign-toplevel-list` lacks it.
   - Minimise is Scale: scale and translate toward the rect plus fade, with `gentle`.
   - Genie is a distinctive mesh warp. Leave it for later, if ever; Scale is the
     brand-neutral choice and costs nothing in the renderer.
5. **Hide** (now). Fade all the app's windows with `fade`; no spatial motion. Unhide is the
   reverse.
6. **App switcher** (now, compositor-drawn or `bande-launcher`; windows.md already specifies
   it).
   - The overlay is delayed by 150 ms so quick switches never show it.
   - The selection ring moves with `quick`.
   - Accept file drops onto icons: with a drag in progress, Super+Tab shows the switcher and the
     drop opens the file there.
7. **Overview** (later, after workspaces).
   - Windows fly from their real rects to a layout that preserves relative positions. Use a
     simple layout: rows by original y, scale to fit, keep order by x.
   - The 3-finger swipe drives progress 0 to 1 through the `SwipeTracker` (see gestures.md).
   - Every window interpolates `lerp(real_rect, overview_rect, progress)`, and the release
     settles with `standard` from the finger velocity.
   - The workspace strip slides in from the top with the same progress.
8. **Full screen** (now, in place per windows.md). Frame springs to the output bounds with
   `gentle`; menu bar and panel slide out with `standard`. With Reduce Motion, cross-fade.
9. **Window menu and keyboard** (now). Every motion above is reachable from the window menu and
   shortcuts; the animation is the same whether triggered by drag, key or menu (DESIGN: one
   obvious way, alternatives lead to the same result).
10. **Stage Manager** (never). It is a second window-management model; the overview plus
    workspaces cover the need.
