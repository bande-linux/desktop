# Window resize zones

## Summary

The resize zone is a 6 pt strip that lies entirely outside the window. Nothing inside the visible edge resizes, and the corner target sits outside the rounded corner's bounding box, away from the curve you can see. Plan: one shared hit test in `unbound-graphics` with a band 8 pt outside plus 3 pt inside that follows the rounded corner, corners reaching 24 pt along each edge, used by both the toolkit chrome and the compositor frame. A compositor fallback band then covers client-side apps that leave no margin of their own. 3 milestones.

## What the owner sees

Aiming at a window's edge or corner often misses. One pixel too far in, the press moves the window (in-house apps) or clicks the app (server-side frames). Too far out, it clicks the window behind. At the bottom-right corner, the spot the eye picks, on the curve, does not resize at all. The owner's display is a 27" 2560×1440 at scale 1 (`~/.config/unbound/config.kdl`, about 109 ppi), so the 6 px band is about 1.4 mm wide.

## Findings

Numbers are in logical px (pt). Window corner radius r = 14 (`Radius::XL`, `crates/toolkit/design/src/radius.rs:8`; the compositor clips every window to it, `crates/platform/compositor/src/scene.rs:26,947-952`).

1. **The band is outside only and thin.**
   - `RESIZE_MARGIN = 6` (`crates/toolkit/graphics/src/title_bar.rs:7`).
   - Toolkit: `Frame::zone_at` returns TitleBar/Content for any point inside the window rect and only reaches resize outside it, up to 6 pt (`crates/toolkit/platform/src/chrome.rs:159-176`). The input region is the window inflated by 6 (`chrome.rs:178-180`, `window.rs:1765-1772`).
   - Compositor (server-side frames): `zone_at` returns `None` for any point inside the frame below the title bar, so it reaches the client, and resizes only within 6 outside (`crates/platform/compositor/src/frame.rs:95-107`).
   - Zero pt inside the visible edge resize.

2. **In toolkit windows a move strip sits right inside the resize strip.** The 6 pt chassis rim around the sheet (`SHEET_INSET = 6`, `chrome.rs:15`) counts as `Zone::TitleBar` (`chrome.rs:166-169`), and a press there starts a move (`window.rs:1570`). Going inward from outside you pass: window behind | 6 pt resize | visible edge | 6 pt move | content. Overshooting by one pixel inward drags the window instead of resizing it.

3. **The corner target is detached from the visible corner.**
   - `resize_edge` calls a point a corner when it lies within `RESIZE_CORNER = 14` of the bounding-box corner along an edge (`title_bar.rs:8,88-111`). That is exactly r, so the corner zone ends where the curve begins.
   - The arc's midpoint is 4.1 pt in from each side, 5.8 pt diagonally from the bounding corner. The transparent wedge between arc and bounding box (about 42 pt² per corner) is inside the rect, so it is never resize:
     - Toolkit windows: the wedge is Zone::TitleBar, so a press there moves the window.
     - Server-side frames: top wedges are the title bar; bottom wedges go to the client surface (`frame.rs:95-99`, `181`).
   - The whole corner target is an L-shaped strip 6 pt wide outside the bounding box, 204 pt² in all. This is the same failure that drew criticism of macOS 26 Tahoe in January 2026: a larger corner radius pushed about 75 % of the 19×19 corner target outside the visible window. On earlier macOS about 62 % of it lay inside ([Daring Fireball](https://daringfireball.net/2026/01/resizing_windows_macos_26), [Michael Tsai](https://mjtsai.com/blog/2026/01/12/the-struggle-of-resizing-windows-on-tahoe/)).

4. **Tiled toolkit windows have no resize band at all.**
   - `resize_margin()` returns 0 when tiled (`window.rs:692-704`), and nothing inside resizes. The shared-edge resize (`tests/headless/tiling.rs:161`) is therefore only reachable by an app's own `xdg_toplevel.resize`.
   - The same function also returns 0 when the swapchain is not premultiplied-alpha (`window.rs:696-700`, `2375`). Such a window has no band anywhere.

5. **Third-party client-side apps get other numbers.**
   - GTK 4 sets its input region to the border box + `RESIZE_HANDLE_SIZE` 12 on every side. Its corners are `RESIZE_HANDLE_CORNER_SIZE` 24, or the corner radius if larger, measured inward from the content edges ([gtkwindow.c](https://gitlab.gnome.org/GNOME/gtk/-/blob/main/gtk/gtkwindow.c)).
   - The compositor clips GTK's own shadow away but keeps its input region, so GTK apps are already easier to resize than ours.
   - Electron apps (VS Code and Claude Desktop are running) and any CSD client without an input margin get nothing from the compositor. `zone_at` returns `None` when `title_bar_height() == 0` (`frame.rs:88-91`).

6. **Cursor feedback is not the cause.**
   - Server-side frames: the shape changes on the same motion event (`frame.rs:213-218`, `input.rs:590-593`).
   - Toolkit: `wp_cursor_shape.set_shape` is sent on motion (`window.rs:1528-1533`, `seat.rs:209-228`) and flushed on every loop pass (`run.rs:123`).
   - The cursor is right wherever the zone is. The zone is the problem.

7. **Scale is not the cause.** All zones are in logical units and the margins are integers, so 1×, 1.5× and 2× behave alike: 6 pt = 6, 9 or 12 device px. The owner runs at 1×, where it is thinnest.

8. **Touchpad.** A clickpad press usually shifts the pointer 1-3 px as the finger rolls, and acceleration makes it hard to stop inside a 6 px strip. Both argue for a wider band that straddles the edge rather than lies beside it.

**Root cause:** a 6 pt band entirely outside the window, nothing inside, a move strip right inside it, and a corner zone sized to the radius that skips the visible curve and its wedge.

**Comparison:**
- Mutter: the draggable border is 10 px in total, visible plus invisible ([mutter 3.1.90 NEWS](https://mi.rro.rs/gnome/sources/mutter/3.1/mutter-3.1.90.news)).
- GTK 4 CSD: 12 outside, corners 24 reaching inward.
- macOS: edges respond slightly outside the window; the corner target is 19×19 and mostly inside before Tahoe.
- KDE: the grab area follows the decoration's border width, and Breeze adds invisible borders only on borderless sides ([KDE bug 307721](https://bugs.kde.org/show_bug.cgi?id=307721)).

## Options

**A. Only widen the outside band (6 → 10).**
- One constant, about 30 min of work.
- Fixes the thinness only. The wedge, the move strip and tiled windows stay broken.
- It also takes more clicks away from the window behind.

**B. Shared hit test that straddles the edge and follows the curve (recommended).**
- The band is 8 pt outside plus 3 pt inside (11 pt total, close to Mutter's 10).
- The inner boundary is the rounded rect inset by 3, so the wedge and the first 3 pt inside the curve resize.
- Corners reach 24 pt from the bounding corner along each edge, GTK's number. With r = 14 and 8 pt outside, the corner target is 32 pt along each edge and 11 pt deep, and it covers the visible curve.
- Trade-offs:
  - Takes 3 pt from the title bar's top edge and from the toolkit rim, which keeps 3 pt of move.
  - On server-side frames it takes 3 pt from the client's outer pixels; a scrollbar flush at the right edge loses its last 3 pt.
  - Tiled windows gain resize from their inner 3 pt, so a seam between two tiles has a 6 pt target.

**C. B plus a compositor fallback band for client-side windows.**
- If a CSD toplevel's own input region does not cover a point within 8 pt outside its geometry, the compositor treats the point as a resize zone.
- The client still wins wherever its input region reaches, so GTK keeps its own 12 pt and nothing changes there.
- Covers Electron apps, toolkit windows without alpha, and any minimal client. The cost is one more branch in `pointer_target`.

**D. Make the bands configurable** (Mutter's `draggable-border-width`). This is YAGNI until someone asks.

## Recommendation

Option C, done as B first and then the fallback:
- Outside band: `RESIZE_MARGIN = 8`.
- Inside band: 3, measured from the rounded outline.
- Corner reach: 24 along each edge.
- Floating windows get the outside band; tiled windows get the inside band only; maximised and full-screen windows get no band.

## Implementation plan

Each milestone is one branch from `main`, gated by fmt, workspace clippy `-D warnings` and tests, and checked live on the mini before merging.

### M1. One rounded, straddling hit test; the toolkit chrome uses it (about 2 h)

- `crates/toolkit/graphics/src/scene.rs`: add `Shape::distance(self, point: Point) -> f32`, the signed distance to the rounded outline (negative inside).
  - The same formula exists twice already: `rounded_distance` (`crates/platform/compositor/src/scene.rs:165`) and `rounded_box` (`crates/shell/screenshot/src/indicator.rs:119`).
  - Switch `indicator.rs` to it, since it is f32 and in the same units.
  - Leave the compositor's f64 physical one alone unless it converts cleanly.
- `crates/toolkit/graphics/src/title_bar.rs`:
  - Replace `resize_edge` and `RESIZE_CORNER` with `resize_zone(window: Rect, radius: f32, outside: f32, point: Point) -> Option<ResizeEdge>`.
  - The point is in the band when it is inside `window.inset(-outside)` and `Shape::rounded(window, radius).distance(point) >= -RESIZE_INSIDE`.
  - It is a corner when it lies within 24 of an end on both axes (the existing `side` logic with the new reach).
  - Set `RESIZE_MARGIN = 8` and add `RESIZE_INSIDE = 3.0`. Make the corner reach private.
  - Drop `RESIZE_CORNER` from `lib.rs:25`.
- `crates/toolkit/platform/src/chrome.rs`:
  - `Frame::zone_at` asks `resize_zone` first, so the band wins over TitleBar and Content.
  - It uses `Radius::XL`, or 0 when the window is squared; `Frame` needs to know whether it is tiled.
  - Remove `RESIZE_BAND`.
- `crates/toolkit/platform/src/window.rs`: `resize_margin()` keeps 0 for tiled windows, but the frame still hit-tests with outside = 0 so the inside band works. Maximised and full-screen windows have no band.
- Tests:
  - `crates/toolkit/graphics/tests/resize.rs`, new. Each case fails on today's code:
    - In the bottom-right wedge (1, 1 in from the bounding corner) → BottomRight.
    - 2 pt inside the arc's midpoint → BottomRight.
    - 2 pt inside the straight right edge → Right; 4 pt inside → None.
    - 7 pt outside → Right; 9 pt outside → None.
    - On the bottom band 20 pt from the corner → BottomRight; 30 pt from it → Bottom.
    - With outside = 0 (tiled), 2 pt inside → edge and 1 pt outside → None.
  - `crates/toolkit/platform/tests/input.rs`, next to `dragging_just_outside_the_right_edge_resizes_the_window` (line 819), end to end through the compositor:
    - A drag from 2 pt inside the right edge widens the window and the content sees no press.
    - A drag from the bottom-right wedge grows both width and height, where today it moves the window.
    - A drag from 4 pt inside the bottom rim still moves the window.
- Docs: `docs/architecture/toolkit.md:142` and `docs/design/features/windows.md:77`, with the new numbers.

### M2. Server-side frames use the same test (about 1-1.5 h)

- `crates/platform/compositor/src/frame.rs`:
  - `zone_at` calls `resize_zone` before the "inside the frame" branch, with `CORNER_RADIUS`, or 0 and outside = 0 for tiled windows (`TILED`, `scene.rs:947`).
  - It returns no band when maximised or full screen.
  - Inside-band points become `Target::Frame`, so the client gets `wl_pointer.leave` there (`input.rs:583-593`).
- Tests in `crates/platform/compositor/tests/headless/decoration.rs`, next to `dragging_the_edge_resizes_the_app_without_its_title_bar` (line 217):
  - A drag from 2 pt inside the content's right edge resizes, and the client sees no button press.
  - A drag from the bottom-right wedge resizes both axes.
  - A press 2 pt inside the top edge of the title bar resizes from Top, not a move.
  - A press 10 pt outside the right edge does not resize.
- Docs: `docs/architecture/compositor.md:55`.

### M3. Compositor fallback band for client-side windows (about 1.5 h)

- `crates/platform/compositor/src/frame.rs`, `pointer_target`:
  - For a `Placed::Window` with `title_bar_height() == 0` that is floating, try `surface_at` first.
  - If no surface takes the point and `resize_zone(geometry, CORNER_RADIUS, RESIZE_MARGIN, point)` is an outside-only hit, return `Target::Frame(surface, Zone::Resize(edge))`.
  - `press_frame` and the cursor already handle `Zone::Resize`.
- Tests in `decoration.rs`, using a `DecorationMode::ClientSide` test client with no input margin:
  - A drag 4 pt outside its right edge resizes it.
  - A client whose input region covers that margin (the GTK case) receives the press itself, and no compositor resize starts.
- Docs: `docs/architecture/compositor.md`, one sentence.

Live check on the mini after each milestone: resize Files, Settings, a terminal, one GTK app and VS Code. Use the mouse at the four corners and edges, the touchpad at the bottom-right, and a pair of tiled windows at their seam.

## Risks

- **Clicks meant for the window behind.** An 8 pt outside band takes 2 pt more from it than today. GTK takes 12.
- **App pixels at the edge.** The 3 pt inside band on server-side frames takes the last 3 pt of app content at the edges, such as an edge-flush scrollbar or a terminal's last column of selection. Mutter and macOS accept the same.
- **Goldens.** The toolkit surface margin grows from 6 to 8. The compositor clips to the geometry, so visible pixels should not change. If a golden moves, it is a placement bug; look at the diff before refreshing it.
- **Top-left corner and Close.** The top-left corner zone overlaps the Close cell's outer 3 pt (the cell reaches x = 0, `docs/architecture/toolkit.md`). The corner wins there; Close's glyph centre (24, 24) stays far away.
- **Fallback band hides real input-region intent.** A client that deliberately leaves a gap outside its geometry gets compositor resize there. Only floating toplevels are affected, never popups or layers.

## Decisions for the owner

1. **Band widths.**
   - 8 pt outside + 3 pt inside is recommended.
   - Alternatively 10 + 3, which matches GTK more closely but steals more from the window behind.
2. **Inside band on third-party server-side windows.** 3 pt is recommended; 0 leaves app pixels alone at the cost of consistency.
3. **Fallback band for CSD apps (M3).** Ship it, or leave Electron and other CSD apps to their own margins until the "GTK apps do not fit our window bar" item settles whether they move to server-side frames.
