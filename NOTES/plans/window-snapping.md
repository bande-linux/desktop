# Window snapping: trigger at the taskbar edge, show the preview at once

## Summary
Drag-to-snap triggers only when the pointer reaches the physical screen edge on the left and right. With the taskbar (rail) on the left, that edge sits behind the taskbar. On top of that, the preview waits 150 ms after the pointer stops, any 2 px wobble restarts the wait, and the outline then takes about 110 ms to grow. The fix: trigger at the usable-area edge (the taskbar edge or anything beyond it), show the preview at once on outer edges, wait 250 ms only on edges shared with another display, and use faster springs.

## What the owner sees
- Dragging a window left over the taskbar does nothing. The pointer must go to x ≤ 1, which is behind the 64 px rail. The tile itself is correct: it starts at the rail edge.
- Once the pointer is at an edge, the outline appears only after the hand is completely still for 150 ms. It then grows from a dot under the pointer. From the moment the pointer stops it takes about 260 ms until the outline looks complete, and longer if the hand drifts.
- On release, the window springs into the tile over about 250 ms.

## Findings
Trigger zone (the "drag to the taskbar" part):
- `crates/platform/compositor/src/tile.rs:136-152` `snap_at`: left and right test the screen rect (`output`, lines 137-138). The top zoom test (line 151) and the corner rows (139-140) already use the usable area (`area`). So the top edge already behaves the way the owner wants (the band reaches from the screen edge through the menu bar to the usable-area edge), and only the sides do not.
- `tile.rs:16` `EDGE_REACH = 1.0`, `tile.rs:17` `CORNER_REACH = 64`.
- `tile.rs:351` `track_snap` passes `screen_rect(screen)` and `usable_area(screen)`. `tile.rs:177-183` `snap_rect` and `tile.rs:186` `tile_window` size the tile from `usable_area`, so the target is already "screen minus taskbar".
- `crates/platform/compositor/src/screen.rs:206-211` `usable_area` = smithay `layer_map_for_output(..).non_exclusive_zone()`, which is the screen minus the layer-shell exclusive zones.
- The default bar is the combined left rail: `crates/services/config/src/lib.rs:535-543` (edge `Left`, menu bar `InBar`), `crates/shell/bar/src/surfaces.rs:58-60`, and `crates/shell/bar/src/combined_client.rs:24-37`, which is anchored left/top/bottom with exclusive zone `bar_width(40) = 40 + 24 = 64` px (`crates/shell/bar/src/rail.rs:19,65`). The owner's config (`~/.config/unbound/config.kdl`) keeps these defaults on a 2560×1440 display at scale 1, so usable x starts at 64.
- `screen.rs:261-273` `nearest_on_screens` clamps the pointer to `0..w-1`. The left trigger therefore needs the pointer at x 0 or 1, under the rail.
- `ExclusiveLayer` (`crates/platform/compositor/src/state/mod.rs:76`) is keyboard exclusivity for layer surfaces, not exclusive zones. It is unrelated.

Timing (the "a bit slow" part):
- `tile.rs:14` `REST = 150 ms` and `tile.rs:15` `REST_DISTANCE = 2.0` px. `tile.rs:359-364`: any move of more than 2 px from the rest point restarts the 150 ms. At a clamped edge, vertical drift alone restarts it. A held mouse wobbles by more than 2 px often, so the real wait is often well over 150 ms.
- `tile.rs:378-381`: the preview is due at `since + REST`. `crates/platform/compositor/src/main.rs:564,579,587` wakes the loop for it, so there is no extra latency beyond REST (event times and `now()` are both CLOCK_MONOTONIC).
- `tile.rs:389-396`: the preview grows from a 1×1 rect at the rest point with `Spring::QUICK`. `crates/libs/spring/src/lib.rs:48,56-60`: QUICK is critically damped with ω = 2π/0.18 s. Remaining distance is (1+ωt)e^(−ωt): 90 % at 111 ms, 99 % at 190 ms. FADE (`lib.rs:54`, 0.12 s) gives 74 ms and 127 ms.
- `tile.rs:427-445` `finish_move`: a drop tiles only if the preview was already shown, so releasing within the dwell does nothing.
- `tile.rs:210`: the drop springs with `Spring::STANDARD` (0.25 s): 90 % at 155 ms, 99 % at 264 ms. The frame moves at once. The buffer is stretched until the client commits (`motion.rs:186-197` targets the pending location and requested size), so it does not wait on the client.
- Root cause of the slowness: a dwell that must be absolutely still (150 ms, restarted by 2 px), followed by a 0.18 s grow from a point. The springs are not the main problem.

Multiple displays:
- `snap_at` does not know about neighbouring screens. A pointer at the shared edge triggers Right on one display, and once it crosses, Left on the next (`x <= output.x + 1`). Only the 150 ms stillness keeps a crossing drag from tiling. With a 0 ms dwell, every crossing would flash a preview.
- The candidate is a bare `Snap`, so Right on display A and Right on display B compare equal (`tile.rs:360`).
- Test displays are placed side by side (`crates/platform/compositor/src/displays.rs:516-531`), and `tests/headless/displays.rs:20-28` has a `two_displays` fixture.

Tests today: `crates/platform/compositor/tests/headless/tiling.rs:58-67` (a sweep without rest does not tile), `:69-84` (drop after rest), `:86-124` (the preview appears only after 150 ms), `:194-229` (reduce motion), `:252-266` (Escape). There is no test with a side layer surface and no multi-display snap test. `tests/headless/support.rs:135-151` has `top_bar`, but no left rail.

Docs: `docs/design/features/windows.md:93-99` specifies the 150 ms rest and "grows from the pointer". `docs/design/catalogue.md:61` says "screen edge".

## How others do it
- KWin (`src/window.cpp`, `checkQuickTilingMaximizationZones`): the band is measured from the usable area (`clientArea(MaximizeArea)`), which excludes panels: `x <= area.x + 20` and `x >= area.right - 20`. Maximize is `y <= area.y + 5`. On an outer edge the outline shows **immediately**. If the edge borders another output (`innerBorder`), it waits for a single-shot **250 ms** timer that only restarts when the mode changes. https://invent.kde.org/plasma/kwin/-/raw/master/src/window.cpp
- GNOME/mutter (`src/compositor/meta-window-drag.c`, `update_move_maybe_tile`): the band runs from the monitor edge to `work_area.x + shake_threshold` (drag-threshold × 6, so 48 px by default). A panel therefore counts as edge. The preview is immediate for a floating window. `TILE_PREVIEW_TIMEOUT_MS 200` applies only when shaking loose an already tiled window, and the timer is never restarted by motion. https://github.com/GNOME/mutter/blob/main/src/compositor/meta-window-drag.c
- Windows 11: a translucent overlay appears when the window is dragged to the edge, with no documented delay. There is also a setting to snap "without dragging all the way to the screen edge". https://support.microsoft.com/en-US/windows/experience/snap-your-windows
- macOS Sequoia: the window tiles when the pointer hits the screen edge and the button is released. Holding Option shows tiles anywhere during a drag. A setting can require Option for edge tiling. https://www.cultofmac.com/how-to/window-snapping-on-mac, https://www.computerworld.com/article/3550352/how-to-use-window-tiling-with-macos-sequoia.html

No mainstream system needs a perfectly still pointer. KWin and mutter measure the band from the panel edge, and only KWin delays, and only on shared edges.

## Options
Trigger zone:
1. **Usable-area edge, with everything beyond it counted (recommended).** Left fires at `x <= area.left + 1`, right at `x >= area.right - 2`. The rail and anything else between the area edge and the screen edge count as edge. The rule matches the top edge today, and both KWin and mutter do the same. On a side without a panel nothing changes.
2. A wider band past the area edge (KWin 20 px, mutter 48 px). This is more forgiving, but it costs a strip of every screen's free edge for window placement and makes accidental tiles more likely.
3. Physical edge only (today). This is the owner's complaint.

Timing:
1. **0 ms on outer edges, 250 ms on shared edges, and a timer that restarts only when the target changes (recommended).** This is the KWin model.
2. Keep a short dwell on outer edges (about 100 ms) but drop the 2 px restart. It guards a little better against dropping by accident near an edge (the reason `windows.md` gave), but it still feels slower than every reference system.
3. A macOS-style modifier (Alt shows tiles anywhere). This is a separate feature and does not address the complaint.

Shared edges:
1. **Hold for 250 ms (recommended).** This is simple, reuses the existing timer, and matches KWin.
2. Edge resistance: the pointer sticks for N px of push before it crosses. This is closer to "pushed", but it needs a pointer barrier that interacts with `nearest_on_screens`, pointer constraints and display arrangement. That is a lot of new code for a rare setup (YAGNI).

## Recommendation
Trigger option 1, timing option 1, shared-edge option 1. Then make the preview spring 0.12 s (FADE) instead of 0.18 s. Make the drop spring 0.18 s (QUICK) instead of 0.25 s if the owner wants it (decision 2). End result: the outline is complete about 75 ms after the pointer touches the taskbar or an outer edge, the window settles about 110 ms (90 %) after release, and crossing to another display does not flash a preview unless held for 250 ms.

## Implementation plan
Each milestone is its own short branch from `main`, gated by fmt, workspace clippy `-D warnings` and workspace tests, then verified live on the mini before merging.

**M1: Trigger at the usable-area edge (about 1 h)**
- `crates/platform/compositor/src/tile.rs`: in `snap_at`, test left and right against `area` instead of `output`. The pointer is always on this screen, so `x <= area.location.x + EDGE_REACH` already covers the strip over the rail. Keep the `output` parameter only if M2 needs it, and delete it otherwise.
- Unit tests in `tile.rs` (`#[cfg(test)] mod tests`, like `grab.rs:468`) for the pure `snap_at`. Use a screen of 0..256 with an area starting at x 32: the pointer at 10 (over the rail) and at 33 gives `Left`; at 36 it gives `None`. With no panel, the right edge at 254 gives `Right`. The corner rows still use the area. These fail on main.
- `tests/headless/support.rs`: add `left_rail(server)`, a shell layer anchored Left|Top|Bottom with size (BAR, 0) and exclusive zone BAR, next to `top_bar`.
- `tests/headless/tiling.rs`: a new test, `a_window_dropped_over_the_left_rail_fills_the_half_beside_it`. With a rail, drag to (10, 96) and to (BAR, 96), drop, and the window is `rect(BAR, 0, (256-BAR)/2, 192)`. Drag to (BAR+8, 96) and it is not tiled.
- `docs/design/catalogue.md:61`: "screen edge" becomes "edge of the free area (the taskbar edge counts)".

**M2: Immediate preview on outer edges, 250 ms on shared edges (about 2 h)**
- `tile.rs`:
  - Remove `REST`, `REST_DISTANCE` and `SnapDrag::rest_at`. Add `SHARED_EDGE_HOLD: Duration = 250 ms`.
  - Make the candidate `(screen, Snap)`, so the same side on another display is a new target.
  - In `track_snap`, restart `since` only when the candidate changes. Pick the delay per candidate: zero unless the physical edge behind the zone borders another screen. Test this with `self.screen_at` on the point one pixel outside the screen rect (left: `(output.x - 1, y)`, right: `(output.right, y)`, top: `(x, output.y - 1)`). Store the delay in `SnapDrag`.
  - `snap_due` shows the preview when `now >= since + delay`, so outer edges show it inside the same motion event. The preview starts from `self.pointer_location`.
- `tests/headless/tiling.rs`:
  - Replace `the_preview_appears_only_once_the_pointer_rests` with `the_preview_appears_as_soon_as_the_pointer_reaches_an_outer_edge`: at t=1000 `snap_tick(1000) == None`, and the preview pixel is lit after `animate(1000)`, `animate(1100)`.
  - Change `drop_after` and `a_window_dropped_after_resting…` to drop with no rest.
  - Turn `a_window_swept_past_the_edge_without_resting_is_not_tiled` into its shared-edge form.
  - Check that the Escape and reduce-motion tests still assert what they meant (they call `snap_tick(1150)`, which is harmless).
- Multi-display tests: move `side_display`/`two_displays` from `tests/headless/displays.rs` into `support.rs` (reuse, not copy), then add to `tiling.rs`:
  - `a_window_dragged_across_to_the_next_display_is_not_tiled`: the pointer at (255, 96) at t=1000, then (300, 96) at t=1100, release. The window is untiled, on the side display.
  - `holding_at_a_shared_edge_for_250_ms_tiles_the_window`: (255, 96) at t=1000, wobble to (254, 99) at 1100 and (255, 92) at 1200. `snap_tick(1200) == Some(1250)`, `snap_tick(1250) == None`, release. Right half of the main display.
  - `the_outer_edge_of_the_side_display_still_previews_at_once`: the pointer at (575, 120). The preview is due at once.
- `docs/design/features/windows.md:93-95`: replace the 150 ms rest with "The outline appears as soon as the pointer reaches the edge of the free area; the taskbar counts as edge. Between two displays the pointer must rest 250 ms, so moving a window across does not tile it."

**M3: Faster springs (about 1 h)**
- `tile.rs:389-396`: preview `Spring::FADE` (0.12 s) instead of `Spring::QUICK`.
- If the owner agrees (decision 2): drag-drop tiles spring with `Spring::QUICK`. `tile_window` takes the spring as a parameter from `finish_move`, and keyboard and menu tiling stay `STANDARD`. If not, skip this part.
- Tests:
  - `tiling.rs`: `the_snap_preview_covers_its_tile_within_80_ms`. Grab, pointer at (0, 96) at t=1000, `animate(1000)`, `animate(1080)`, then render. Pixel (100, 10) is lit. The first tick counts one refresh (`motion.rs:27-31`), so this is about 96 ms of spring. FADE is 95 % of the way there, so the preview's top edge is at y ≈ 5. QUICK is 85 %, so the top edge is at y ≈ 14 and the test fails.
  - `motion.rs`: if the drop spring changes, `a_window_dropped_on_a_tile_is_nearly_in_place_after_120_ms` uses `reddish` bounds within 4 px of the tile at release+120 ms, which is about 136 ms of spring. QUICK leaves 5 % of the distance, STANDARD 14 %. Choose the start point so the drop travels at least 60 px, which makes STANDARD fail.
- `windows.md:99`: update the motion line.

**Live check (part of each milestone, on the mini):** build there, restart the session with the owner's warning first (see "warn before visible disruption"), then drag windows onto the rail, the right edge and the top. With a second display connected, check the shared edge too. Otherwise the headless tests cover it.

## Risks
- With no dwell on outer edges, every drop with the pointer over the taskbar or at an edge tiles. Pushing a window half off the left edge then needs a grab point further right on the title bar. Every reference system behaves this way. Escape during the drag still cancels (`grab.rs:240-281`).
- Auto-hide or full-screen sets the rail's exclusive zone to 0 (`crates/shell/bar/src/dock.rs:691-698`). The area is then the full screen and the band falls back to the physical edge. That is correct, but it is a second behaviour to remember when testing.
- Shared-edge detection uses `screen_at` on the point just outside the screen. Displays that touch only partly (offset vertically) count as shared only where they overlap, which is the right behaviour. Mirrored screens are skipped by `screen_at`.
- The event-time ms counter wraps after 49.7 days of uptime while `now()` does not. This is a pre-existing issue that only affects the 250 ms shared-edge hold.
- Changing `snap_at`'s band also changes Ctrl+Super+drag-anywhere moves (same path). That is intended.

## Decisions for the owner
1. Outer edges: preview immediately (recommended, like Windows, GNOME and KDE), or keep a short 100 ms dwell that no longer resets on wobble?
2. Should the window's spring into its tile after a drop also get faster, 0.18 s instead of 0.25 s? Keyboard and menu tiling would stay at 0.25 s.
