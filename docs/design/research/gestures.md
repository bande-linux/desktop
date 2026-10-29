# Trackpad gestures and scrolling

Research date: 2026-10-29. Sources are linked inline; "unverified" marks what we could not
check.

## What macOS does

### The gesture set

Apple's list, from <https://support.apple.com/en-us/102482>:

| Fingers | Gesture | Action |
|---|---|---|
| 1 | Tap | Click (tap to click) |
| 2 | Click or tap | Secondary click |
| 2 | Double-tap | Smart zoom (web page, PDF) |
| 2 | Slide | Scroll |
| 2 | Pinch | Zoom |
| 2 | Rotate | Rotate |
| 2 | Swipe left/right | Back and forward between pages |
| 2 | Swipe left from the right edge | Notification Center |
| 3 | Drag | Three-finger drag; off by default, in Accessibility |
| 3 | Tap, or force click | Look up and data detectors |
| Thumb + 3 | Spread | Show Desktop |
| Thumb + 3 | Pinch | Launchpad / Apps |
| 4 (or 3) | Swipe up | Mission Control |
| 4 (or 3) | Swipe down | App Exposé |
| 4 (or 3) | Swipe left/right | Between full-screen apps and Spaces |

Apple states the 3 vs 4 finger choice per gesture as a setting. The page gives no thresholds
or tracking behaviour.

### How it feels

macOS publishes no numbers for this; everything below is behaviour plus Apple's general
principles.

- **What tracks 1:1:**
  - Scrolling, pinch zoom, rotate, the Spaces swipe and two-finger page swipes in Safari all
    follow the fingers continuously, and can be reversed before lifting.
  - Mission Control and App Exposé follow the vertical swipe progressively in recent macOS
    (unverified since which version).
  - The page swipe uses `NSEvent.trackSwipeEvent(options:dampenAmountThresholdMin:max:)`,
    which reports a gesture amount in "pages" with dampening past min and max: rubber band
    built into the API.
    <https://developer.apple.com/documentation/appkit/nsevent/trackswipeevent(options:dampenamountthresholdmin:max:usinghandler:)>
- **Momentum is part of the event stream.** Scroll events carry `phase` (fingers on) and
  `momentumPhase` (system-generated inertia after lift-off). A view can tell finger scrolling
  from inertia and stop or redirect it.
  <https://developer.apple.com/documentation/appkit/nsevent/momentumphase>
  macOS generates the inertia centrally, so every app's scrolling decays identically. This is
  the single biggest reason scrolling feels the same everywhere on a Mac.
- **Velocity hand-off and projection:**
  - On release, the remaining motion starts at the finger's velocity.
  - The end point is projected with the scroll deceleration rate. UIKit's
    `DecelerationRate` is about 0.998 per ms (normal) and 0.99 (fast); exact values unverified.
    <https://developer.apple.com/documentation/uikit/uiscrollview/decelerationrate-swift.struct>
  - Then the system snaps to the nearest valid target: the next Space, or back.
  - <https://developer.apple.com/videos/play/wwdc2018/803/>
- **Rubber band:**
  - Past the first or last Space, or the end of a scroll view, content follows with growing
    resistance and springs back on release.
  - The commonly cited UIKit curve is `f(x) = d · c · x / (d + c · x)`, with `d` the viewport
    size and `c = 0.55`.
    <https://www.holko.pl/2014/07/06/inertia-bouncing-rubber-banding-uikit-dynamics/>
  - WebKit and Kivy use the same constant.
    <https://kivy.org/doc/master/_modules/kivy/effects/dampedscroll.html>
  - No Apple primary source, so unverified.
- **Interrupting:**
  - Touching the trackpad during inertia stops it at once (the hold).
  - Starting a new Spaces swipe during the settle animation grabs the animation where it is.
  - Both follow "everything is redirectable" (WWDC18).
- **Thresholds:**
  - There is a small hysteresis before a drag or swipe commits to an axis. Apple cites about
    10 pt on iOS (WWDC18).
  - A swipe commits either by distance (past about half a page) or by velocity (a short
    flick); the exact rule is unverified.
- **Force click:** a second, deeper click with a haptic. It triggers Look Up on words, previews
  links and spring-loads folders during a drag
  (<https://developer.apple.com/documentation/appkit/nsspringloadingdestination>). We have no
  pressure hardware on most Linux trackpads, so three-finger tap is our path.

## Linux and Wayland reality

- **libinput gestures** (<https://wayland.freedesktop.org/libinput/doc/latest/gestures.html>):
  - The gesture types:
    - Swipe: 3+ fingers, centre dx/dy.
    - Pinch: 2+ fingers, scale and rotation.
    - Hold: 1+ fingers resting.
  - Changing the finger count cancels a gesture.
  - On two-slot touchpads a 3-finger pinch can look like a swipe; libinput then reports a
    swipe.
  - Hold exists precisely to stop kinetic scrolling.
- **libinput scrolling** (<https://wayland.freedesktop.org/libinput/doc/latest/scrolling.html>):
  - libinput does **not** implement kinetic scrolling. The caller should compute velocity for
    finger-source scrolls, continue with friction after lift-off, and cancel on leave or key
    press.
  - On Wayland the end of a finger scroll arrives as `wl_pointer.axis_stop`, with
    `axis_source` = finger. <https://wayland.app/protocols/wayland#wl_pointer>
- **pointer-gestures-unstable-v1, version 3**
  (<https://wayland.app/protocols/pointer-gestures-unstable-v1>):
  - Events:
    - Swipe `begin(fingers)`, `update(dx, dy)`, `end(cancelled)`.
    - Pinch adds absolute `scale` and relative `rotation`.
    - Hold has begin and end only.
  - One gesture per seat at a time.
  - Which gestures the compositor keeps is up to the compositor.
- **GNOME `SwipeTracker`** (<https://raw.githubusercontent.com/GNOME/gnome-shell/main/js/ui/swipeTracker.js>):
  the most complete public reference for 1:1 workspace swipes.
  - A full page is 400 touchpad units horizontally and 300 vertically.
  - 16 px before the axis locks.
  - Velocity is sampled over the last 150 ms.
  - Velocity thresholds: 0.6 (touchpad) and 0.3 (touch).
  - Deceleration 0.997 per ms on a touchpad.
  - Release logic:
    - Below the threshold, snap to the nearest page.
    - Above it, project with the deceleration and go to the next page in the direction of
      motion.
  - Duration is the remaining distance / velocity × 3, clamped between 100 ms and
    400 ms × log2(1 + pages crossed).
  - GNOME uses eased durations here, not springs.
- **niri** (<https://github.com/YaLTeR/niri/wiki/Gestures>):
  - 3-finger vertical swipe switches workspaces; horizontal swipe scrolls the view.
  - 4-finger vertical toggles the overview.
  - Release hands velocity to a spring: workspace switch ζ 1.0, k 1000.
  - The closest design to ours.
- **KWin (Plasma 6):** 3 and 4-finger horizontal swipes switch desktops; 4-finger up opens
  Overview. Defaults are hard-coded. Source: forum threads, partly unverified:
  <https://discuss.kde.org/t/plasma-6-wayland-new-gesture-overview-3-fingers/14956>
- **Hyprland:**
  - Since 0.51, `gesture = fingers, direction, action` declarations track 1:1.
    <https://wiki.hypr.land/0.53.0/Configuring/Gestures/>
  - Older settings were swipe distance 300 px, cancel ratio 0.5 and min force speed 30.
    <https://wiki.hypr.land/0.17.0beta/Configuring/Variables>
- **What they do better than macOS:**
  - niri and GNOME expose their numbers and are reproducible.
  - GNOME's duration scaling by pages crossed makes multi-page flicks feel fast without making
    one-page settles abrupt.

## Current state in Unbound

- `crates/platform/compositor/src/input.rs` handles keys, pointer motion, buttons and axis
  frames. **libinput gesture events fall through to `_ => return Ok(())` and are dropped.**
- `grab.rs` already forwards `gesture_*` through Smithay's pointer handle, but no
  `PointerGesturesState` global is created, so no client can receive them.
  `zwp_pointer_gestures_v1` is listed as P1, not implemented (`docs/architecture/compositor.md`).
- The toolkit seat (`crates/toolkit/platform/src/seat.rs`) accumulates `wl_pointer.axis` only.
  It ignores `axis_source` and `axis_stop`, so it cannot tell a finger scroll from a wheel and
  cannot start or stop inertia. There is no scroll view widget yet.

## For Unbound

1. **Compositor gesture router** (now, `crates/platform/compositor`).
   - Handle `InputEvent::GestureSwipe*`, `GesturePinch*` and `GestureHold*`. The compositor
     keeps:
     - 3-finger horizontal swipe: workspaces.
     - 3-finger vertical swipe: overview up, app windows down.
     - Thumb + 3 pinch and spread: launcher and show desktop.
   - It forwards everything else (2-finger pinch and rotate, and holds) to the focused client
     through `zwp_pointer_gestures_v1` v3.
   - Make 3 vs 4 fingers one setting only if users ask; start with 3, as catalogue §7 says.
     4-finger swipes go to clients, which also frees 3-finger drag later.
2. **A `SwipeTracker` in Rust** (now, compositor; one implementation shared by workspace,
   overview and show-desktop).
   - Progress = accumulated delta / distance, with distance 350 libinput units per page. GNOME
     uses 400 and Hyprland 300; tune on Apple Silicon trackpads.
   - Lock the axis after 16 units.
   - Velocity is a least-squares fit over the last 100 ms of updates, not the last delta; the
     last delta is noisy at lift-off.
   - Past the first or last page, apply the rubber-band curve with `c = 0.55` and `d` = one
     page.
   - On end: `target = round(project(progress, velocity, 0.997))`, clamped to one page from
     the start page for a single swipe. Then animate with `momentum` if
     |v| > 0.6 units/ms and with `standard` otherwise, starting at the finger's velocity.
   - Cancelled (finger count changed): animate back with `standard`.
   - A new `begin` during the settle animation takes over from the current progress and
     velocity, never from the target.
3. **Kinetic scrolling in the toolkit** (now, `crates/toolkit/platform` plus a `ScrollView` in
   `crates/toolkit/widgets`).
   - Read `axis_source`, `axis_stop` and `axis_value120`.
   - Finger source: content tracks 1:1. On `axis_stop`, start `decay` at 0.998 per ms from
     the fitted velocity.
   - Wheel source: animate each detent with `quick` to the accumulated target, so fast wheel
     spins merge.
   - At the ends: rubber band during finger tracking and during inertia, which overshoots by
     the projected excess and is capped. Return with `standard`.
   - A `hold` gesture (pointer-gestures v3) or any new finger contact stops inertia instantly.
   - Do not do inertia in the compositor; it does not know content bounds. macOS does it
     centrally, but `axis_stop` plus a shared toolkit gives the same consistency for our apps.
4. **Pinch zoom and smart zoom in apps** (later). Toolkit `PinchGesture` with absolute scale,
   anchor at the gesture centre, rubber band at zoom limits, and `standard` back to the limit.
   Smart zoom needs 2-finger double-tap, which libinput does not report; leave it out.
5. **Back and forward swipe** (later). 2-finger horizontal scroll on a page-based view (Files
   history, Settings panes) becomes a 1:1 page swipe when the content cannot scroll further
   horizontally. This is the `trackSwipeEvent` model.
6. **Notification Center from the right edge** (later). Libinput gives no edge-origin for
   2-finger scrolls. Use a 3-finger swipe left instead, or skip it. The menu bar clock opens it.
7. **Three-finger drag and tap-to-click.** These are libinput configuration
   (`tap`, `drag`, `drag-lock`, `3fg-drag` since libinput 1.27, unverified version). Expose
   tap to click on by default and three-finger drag in Settings > Accessibility (now; the
   compositor sets device config on add).
8. **Look up** (later). libinput turns a 3-finger tap into a middle click
   (tap button map `lrm`, unverified default). Keep middle click, which Linux users rely on
   for primary paste. Offer Look Up from the text context menu and `Super+Ctrl+D` instead.
