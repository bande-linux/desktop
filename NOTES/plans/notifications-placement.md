# Where notifications, the Notification Center and Control Center open

## Summary

On the left taskbar (the default), banners appear at the top right and slide off to the right. The Notification Center and Control Center open at the bottom left, beside the clock. A banner put away to the right turns up on the other side of the screen. Every reference desktop shows banners next to the clock that opens their history. Recommendation: one rule, **"everything opens from the clock corner"**. Banners, the Notification Center and Control Center all open beside the taskbar, in the corner that holds the clock, and slide out of the taskbar's edge and back into it. The trigger stays the clock.

## What the owner sees

On the mini (one 2560×1440 display at scale 1, `bar { … }` with the default left edge, combined bar):

- A banner slides in from the right screen edge and rests in the top right corner (344 pt wide, 12 pt from the edge). Swiping it right or letting it time out sends it off to the right.
- The unread dot then shows on the clock, at the bottom of the left taskbar. Clicking the clock slides the Notification Center out from under the taskbar's edge: a 368 pt full-height sheet beside the taskbar on the left. That is "the notifications bar opens up under the taskbar".
- Control Center (the 2×2 status button just above the clock) also opens at the bottom left, beside the taskbar, on a higher layer than the Notification Center. With both open, Control Center covers the bottom of the Notification Center.
- So notifications arrive on the right and are kept on the left. Their trigger, the clock, is on the left too.

## Findings

1. **Banners always sit at the top right, whatever the taskbar edge.**
   - The banner column is `self.size.0 - MARGIN - BANNER_WIDTH` (`crates/shell/notifications/src/lib.rs:667,677`), stacked down from `y = MARGIN` (`:323`).
   - Banners enter and leave towards positive x (`hidden_offset`, `:144`, `:341`, `:360`), the swipe to put away is to the right (`:959-990`), and the fade only counts a rightward shift (`:1074-1075`).
   - The anchor is never used for banners. The test `a_banner_slides_in_from_the_right_edge_and_rests_at_the_top_right` (`crates/shell/notifications/tests/notifications.rs:81`) pins this.
2. **The Notification Center already follows the taskbar.**
   - `panel_frame` (`lib.rs:546-570`): left edge gives a full-height sheet beside the taskbar; right is the mirror; bottom and top give a sheet right-aligned with the clock; no anchor gives the top right.
   - On the left it slides in from `-(x + CENTER_WIDTH + CLOSE_REACH)` (`:572-578`), so it appears from under the taskbar's edge.
   - Test: `a_left_bar_opens_the_center_beside_its_clock_instead_of_at_the_right` (`notifications.rs:316`). The 12 pt gap (`MARGIN`, `lib.rs:32`) differs from Control Center's 8 pt (`crates/shell/controlcenter/src/lib.rs:35`) and from bar.md's 8 pt.
3. **Control Center follows the taskbar too.** `place_beside` and `slide_from` (`controlcenter/src/lib.rs:51-75`) put it beside the item. It is bottom-aligned on a side bar and slides out of the bar's edge on every edge. It is an Overlay layer surface (`controlcenter/src/client.rs:878`). Notifications are on Top (`notifications/src/client.rs:371-377`), the same layer as the taskbar (`bar/src/combined_client.rs:24-37`).
4. **Only the left edge is combined today.**
   - `MenuBarPlacement::combines` is true only for `InBar` on the left (`crates/services/config/src/lib.rs:142-144`). Any other edge, or `menu-bar "separate"`, keeps the clock in the 24 pt top menu bar (`bar/src/menu_bar.rs:146` reports `Top`).
   - So bottom, right and top taskbars already behave like macOS: clock, Control Center, centre and banners all at the top right. Only the default layout is split. A horizontal combined bar (bar.md §5.4 step 7) is not built yet.
5. **The design chose this split on purpose.** bar.md §3.5 says banners stay "at the top of the far side (top right for a left bar), where they cover the least work; when put away they fly toward the clock". The fly-to-clock part was never built: banners leave to the right. §6 records it as the one break of "everything from the bar", because banners beside the bar would cover the title bars of left-tiled windows. Window controls are at the top left (DESIGN.md §6), so a top-left banner would cover them. A bottom-left one does not.
6. **The clock and Control Center take different paths.**
   - Control Center goes through the compositor: `toggle_control_center` request, then a `toggle_panel` event to every shell client (`compositor/src/shell.rs:433-435`, `:149-151`).
   - The clock calls an in-process closure, `Services::notification_center` (`bar/src/menu_bar_client.rs:399`, `bar/src/surfaces.rs:23,31,48`, `shell/src/shell.rs:49,63-70,95`). `Alt+Super+N` uses the compositor path (`compositor/src/input.rs:299-301`).
   - Nothing closes one panel when the other opens. The live sweep encodes "the newest takes the keyboard" (`bar/tests/live.rs:633-642`) and "a Control Center press while the launcher is open does nothing" (`:635`). These are the two open owner decisions in `NOTES/resume-2026-10-31.md:52-55`.
7. **Do Not Disturb is only in Control Center** (`controlcenter/src/lib.rs:218-226`). The centre shows a line while it is on (`notifications/src/lib.rs:408-411`).
   - bar.md §3.1 promises a clock right-click with Date & Time settings and a Do Not Disturb toggle. No such menu exists: a right click on the clock opens the centre (`live.rs:609`).
   - The clock shows the unread dot even while Do Not Disturb is on. The service already carries `do_not_disturb` in its presentation (`services/notify/src/center.rs:394`), and the bar follows that presentation for the dot (`libs/status/src/unread.rs`).
8. **More than one display.**
   - Only the main display's bar reports anchors (`menu_bar_client.rs:121-123`). The notifications client ignores the anchor's `output` (`notifications/src/client.rs:245-251`). Both panel surfaces open on the default output (`toolkit/platform/src/window.rs:468-475`).
   - So a clock click on a second display opens the centre on the first. This does not affect the owner today: the mini has one display.
9. **Full screen and quiet are fine.** Banners go quiet only on the display in use when it shows a full-screen app (`notifications.rs:383`), and on Do Not Disturb or screen sharing (`center.rs:302-304`). Placement does not change this.

## Prior art

| System | Banners | History / centre | Trigger | Moves with the bar? |
|---|---|---|---|---|
| macOS | top right, slide in from the right edge, swipe right | Notification Center slides in from the right | clock in the menu bar (top right) | no movable bar; all in the clock corner |
| Windows 11 | bottom right, above the clock | notification centre plus calendar above the clock | clock in the taskbar | taskbar flyouts open from its corner |
| Windows 10 | reportedly on the taskbar's side | Action Center | tray icon by the clock | reportedly yes [2] |
| GNOME | top centre, under the clock | message list plus calendar dropping from the clock | top-bar clock | no |
| KDE Plasma | "near notification icon" by default: out of the panel, on the tray's half | tray applet popup | tray icon | yes [3] |
| ChromeOS | above the status area | notification bubble anchored to the shelf corner | status area (clock) | the status area moves with the shelf [4] |

In every system, banners appear near the clock that opens their history, and the history opens from that clock. Unbound's left taskbar is the only layout where banners and their history sit in opposite corners.

## Options

- **A. Keep it (bar.md as written) and build the missing "fly toward the clock".** Banners stay at the top right and fly across the screen into the bottom-left clock when put away. Cheapest. But the split stays: the eye has to cross 2,400 px from a banner to its history, and a swipe to the right goes the "wrong" way.
- **B. Everything at the top right (literal macOS).** The centre and Control Center open on the right edge, far from their triggers. A click at the bottom left opens a panel 2,400 px away, which breaks bar.md's "panels slide out of the item that owns them". It would need a second trigger on the right, such as an edge swipe.
- **C. Everything from the clock corner (recommended).** Banners, centre and Control Center open beside the taskbar, in the clock's corner, and move out of and back into the taskbar's edge.
  - Left taskbar: banners stack up from the bottom left, beside the taskbar, with the newest nearest the clock. They slide out from under the taskbar and are swiped left back into it. They never cover the window controls at the top left.
  - Right taskbar: the mirror image.
  - Bottom taskbar: bottom right above the clock, entering from the right edge (Windows 11).
  - Top taskbar or separate menu bar: top right under the clock, entering from the right edge (macOS, unchanged).

  Costs: banners cover the bottom left of the work area, where a terminal prompt or status bar lives. They are transient (5 s), and resting the pointer holds them. Most of the work is banner geometry in one file.
- **D. Fold notifications into the taskbar** (a list in the expanded 248 pt taskbar, or a notification area in the rail). At 64 pt the rail cannot show text. In the expanded taskbar it would duplicate the centre and push the apps out. The unread dot on the clock is the right amount of integration. Rejected.
- **E. A separate notifications window on the left.** It is the centre again under another name. Rejected.

## Recommendation

Option C, as one rule written into bar.md §3.5:

1. **Place.** Banners, the Notification Center and Control Center open beside the taskbar, in the corner that holds the clock, aligned with their item. Banners and the centre share one column. On a side taskbar the column runs full height; banners stack from the clock's end, newest nearest the clock.
2. **Motion.** Each one slides along the axis away from the taskbar's edge, out of it and back into it: a side taskbar slides horizontally. On a top or bottom taskbar, banners and the centre enter from the right screen edge (the clock's side), as they do today. Control Center keeps its short vertical slide. Putting a banner away (swipe, flick or timeout) sends it back the way it came, towards the clock. Reduce Motion cross-fades, as today.
3. **Trigger.** The clock opens the centre, the Control Center button opens Control Center, and `Alt+Super+N`/`Alt+Super+C` are unchanged. A right click on the clock opens a small menu: Do Not Disturb, Notifications Settings…, Date & Time Settings…. While Do Not Disturb is on, the clock shows a muted moon in place of the unread dot.
4. **One panel at a time.** Opening the centre closes Control Center and the other way round, as on macOS. Banners on screen are put away into the centre when Control Center opens.

Naming: the UI says "taskbar" (per `keep-in-taskbar.md`). Code keeps `bar`, `Panel::NotificationCenter` and `Panel::ControlCenter`.

## Implementation plan

Each milestone is one short branch from `main`, gated by fmt, workspace clippy `-D warnings` and workspace tests, then checked live on the mini.

1. **Banners take their column from the clock's anchor** (~2 h)
   - `crates/shell/notifications/src/lib.rs`: derive the banner column from `self.anchor` next to `panel_frame`. It gives the x of the rest position, the entering side (left of the column on a left edge, right otherwise) and the vertical origin (from the bottom when the clock is on a left, right or bottom taskbar, from the top otherwise).
   - Use it in `rebuild_banners` (stack upward from the clock's bottom when needed), `banner_origin`, `input_region` and `hidden_offset`.
   - `set_anchor` re-lays out banners already on screen. The centre's gap becomes Control Center's `EDGE` (8 pt) so both line up.
   - Tests in `crates/shell/notifications/tests/notifications.rs`:
     - `a_left_bar_shows_banners_beside_it_with_the_newest_nearest_the_clock`: anchor `Left, (-64, 724, 64, 52)`, two stacks. The resting regions start 8 pt right of 0, the newest lies lowest and ends at the clock's bottom, and the first frame is left of the rest position.
     - `a_bottom_bar_stacks_banners_up_from_its_clock_at_the_right`.
     - Keep `a_banner_slides_in_from_the_right_edge_and_rests_at_the_top_right` unchanged for no anchor, and add a `Top` anchor case to it.
     - Update `a_left_bar_opens_the_center_beside_its_clock_instead_of_at_the_right` for 8 pt.

2. **Putting a banner away goes back towards the clock** (~1.5 h)
   - `lib.rs` `release` (`:959-990`) and the drag fade (`:1074-1075`) use the column's put-away direction (left for a left taskbar), and timeouts leave the same way. Rubber-band the wrong direction as today.
   - New golden `banners-left-light.png` in `crates/shell/notifications/tests/golden.rs`: a left anchor, two stacks.
   - Tests:
     - `swiping_a_banner_towards_a_left_bar_puts_it_away_and_away_from_it_springs_back`, with the same thresholds as the existing right-swipe test (`notifications.rs:185`), which stays for the top-right case.
     - `a_banner_that_times_out_on_a_left_bar_leaves_towards_it`.
   - Docs: bar.md §3.5 (the rule above) and §6 (drop "banners at the far corner"), and `docs/architecture/behaviour.md:34` ("top right below the menu bar" becomes the clock-corner rule).

3. **One panel at a time, both through the compositor** (~2 h)
   - `crates/platform/protocols/protocols/unbound-shell-v1.xml`: replace the request `toggle_control_center` with `toggle_panel(panel)`. The protocol is at version 1 with no version checks, so it is changed in place.
   - `crates/platform/compositor/src/shell.rs:433`: call `toggle_panel(panel)`. `crates/toolkit/platform/src/shell.rs`: `Window::toggle_panel(Panel)` in place of `toggle_control_center`.
   - `bar/src/menu_bar_client.rs:396-399`: the clock calls `toggle_panel(NotificationCenter)`. Delete the in-process closure: `NotificationCenter` and `Services::notification_center` (`bar/src/surfaces.rs`), `Context::notification_center` (`shell/src/shell.rs`), `Notifications::toggle`, `Message::Toggle` (`notifications/src/client.rs`), and its use in `bar/tests/live.rs:203`.
   - Control Center's client dismisses itself on `TogglePanel(NotificationCenter)`. The notifications client closes the centre and puts banners away on `TogglePanel(ControlCenter)` (new `Screen::make_way(now)`).
   - Tests:
     - `crates/platform/compositor/tests/headless/shell.rs`: the bar's `toggle_panel(notification_center)` reaches every shell client as the `toggle_panel` event. Add the request helper in `crates/testing/test-wayland/src/shell.rs`.
     - `notifications.rs`: `make_way` closes an open centre, removes banners from the input region, and keeps the notifications in the centre.
     - `bar/tests/live.rs`: `the_clock_over_control_center_closes_it` and `control_center_over_the_notification_center_closes_it`, modelled on `a_bar_menu_opened_over_control_center_closes_it` (`:752`): open one, press the other's item, expect it, then Escape and `await_calm()`. Change `another()` (`:633-642`) to match.
   - Docs: behaviour.md:36 and the resolved line in `NOTES/resume-2026-10-31.md`.

4. **Clock menu and the Do Not Disturb mark** (~1.5-2 h)
   - `bar/src/menu_bar_client.rs`: a right click (and Hold) on the clock opens a local menu with Do Not Disturb (checkable), Notifications Settings… and Date & Time Settings…. The two settings items use `unbound-settings <pane>`, as `:180` already does.
   - The toggle sets the `DoNotDisturb` property through `unbound_notify::client`, as Control Center does (`controlcenter/src/client.rs:359-361`).
   - `crates/libs/status/src/unread.rs`: follow `(unread, do_not_disturb)` from the same presentation. `bar/src/rail.rs` and `menu_bar.rs` draw a muted moon in place of the dot while it is on, and the accessible label says so (`clock-unread` gets a sibling in `bar/locales/en-US/unbound-bar.ftl`).
   - Tests:
     - `bar/tests/live.rs` `shared_cases`: `Clock, Secondary` and `Clock, Hold` expect `Menu`.
     - `bar/tests/rail.rs`: the clock's node name says "Do Not Disturb" when set, and the rail goldens gain a moon case. A unit test in `libs/status` covers the `(unread, quiet)` mapping from a presentation.
     - Run the gate's live test on the mini, toggling from the menu and seeing Control Center's tile follow.

5. **Optional, later: panels on the display whose clock was clicked** (~2 h, only with more than one display)
   - Bars on every display report anchors (drop the main-only return at `menu_bar_client.rs:121`). The notifications and Control Center clients keep anchors per output and open on the output named in the latest `toggle_panel` (add `output` to that event). Banners stay on the main display unless the owner decides otherwise.
   - Test: a headless two-output case in `compositor/tests/headless/shell.rs`, and a `Screen` test that the centre uses the anchor of the requested output.

## Risks

- **Bottom-left banners cover a terminal prompt or a status bar** for up to 5 s. Resting the pointer holds them, and a left swipe clears them. If the owner dislikes it in daily use, milestone 1's column function is the single place to change. Moving to "top, beside the taskbar" would cover the window controls, so the fallback is option A, not top-left.
- **A banner arriving while Control Center is open** lies under it (Overlay is above Top) until Control Center closes. That is acceptable, since banners also wait while the centre is open, but it is not animated.
- **Protocol change.** The bar, compositor and shell ship together, so changing a v1 request in place is safe. Any out-of-tree client using `toggle_control_center` would break; none is known.
- **The live sweep is broad.** Changing `another()` and the clock cases touches every row of the sweep, so run both compact and expanded on the mini before merging.
- **A horizontal combined taskbar is not built.** The bottom rule is only covered by `Screen` tests until bar.md §5.4 step 7 lands.

## Decisions for the owner

1. **Placement model.** Clock corner (C, recommended), banners stay top right (A), or everything top right (B).
2. **Clicking the clock while Control Center is open.** Recommended: close Control Center and open the centre (one panel at a time), rather than "the newest takes the keyboard".
3. **A taskbar click while the launcher is open.** Recommended: the clock and the Control Center button close the launcher *and* open their panel. Other taskbar targets keep "the click only closes the launcher". Milestone 3 does this only on a yes.
4. **Right click on the clock.** Recommended: a menu (Do Not Disturb, Notifications Settings…, Date & Time Settings…) as bar.md §3.1 already says, in place of opening the centre.
5. **Several displays** (milestone 5, later). Banners on the main display, or on the display with the active window.

Sources:
1. Apple, "Use Notification Center on Mac" and Microsoft, "Notifications and Do Not Disturb in Windows" (cited from knowledge, not re-fetched); GNOME Shell top-bar calendar and message list (from knowledge).
2. Vivaldi forum, ["Notifications on the right of the screen"](https://forum.vivaldi.net/topic/78451/notifications-on-the-right-of-the-screen): users report Windows notifications appear on the taskbar's side (anecdotal).
3. KDE, ["Position popups more relative to notification icon"](https://invent.kde.org/plasma/plasma-workspace/-/merge_requests/5): popups emerge from the panel on the tray's half; vertical panels switch top or bottom by screen half.
4. Chromium `ash/system/notification_center/notification_center_bubble.cc` (`anchor_to_shelf_corner`), seen through a search-result mirror and not verified against current source.
