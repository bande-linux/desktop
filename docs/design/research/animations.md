# Animations and motion presets

Research date: 2026-10-29. Sources are linked inline. "Unverified" marks a claim that we could
not check against a primary source. Apple publishes almost no durations for macOS system
animations. Where a number is missing we say so and propose one.

## What Apple actually publishes

### The spring model

- **Springs are described by duration and bounce, not by mass, stiffness and damping.** In
  WWDC23 "Animate with springs", `duration` is a perceptual duration and `bounce` runs from
  -1 to 1. Bounce 0 is critically damped, above 0 overshoots and below 0 is overdamped.
  Guidance: about 0.15 is a small bounce, about 0.3 is noticeably bouncy, and above about 0.4
  "may feel too exaggerated for a UI element".
  <https://developer.apple.com/videos/play/wwdc2023/10158/>
- **Conversion** (same talk). Mass is 1 and stiffness is `(2π / duration)²`. For bounce ≥ 0,
  damping is `4π (1 − bounce) / duration`. In terms of our crate (`ω`, `ζ`):
  **ω = 2π / duration** and **ζ = 1 − bounce**. The exact damping formula was garbled in the
  transcript we read and is derived here from the standard form, so treat it as unverified.
  The `ω` and `ζ` mapping follows from it.
- **SwiftUI presets**, all from the doc JSON, verified:

  | Preset | Declaration |
  |---|---|
  | `smooth` | `smooth(duration: 0.5, extraBounce: 0)`, base bounce 0 <https://developer.apple.com/documentation/swiftui/animation/smooth(duration:extrabounce:)> |
  | `snappy` | `snappy(duration: 0.5, extraBounce: 0)`, base bounce 0.15 <https://developer.apple.com/documentation/swiftui/animation/snappy(duration:extrabounce:)> |
  | `bouncy` | `bouncy(duration: 0.5, extraBounce: 0)`, base bounce 0.3 <https://developer.apple.com/documentation/swiftui/animation/bouncy(duration:extrabounce:)> |
  | `spring` | `spring(response: 0.5, dampingFraction: 0.825, blendDuration: 0)`. A new spring on the same property replaces the old one and **keeps its velocity** <https://developer.apple.com/documentation/swiftui/animation/spring(response:dampingfraction:blendduration:)> |
  | `interactiveSpring` | `interactiveSpring(response: 0.15, dampingFraction: 0.86, blendDuration: 0.25)`, "intended for driving interactive animations" <https://developer.apple.com/documentation/swiftui/animation/interactivespring(response:dampingfraction:blendduration:)> |

- Core Animation has the same model, with `CASpringAnimation(perceptualDuration:bounce:)`
  alongside mass, stiffness and damping.
  <https://developer.apple.com/documentation/quartzcore/caspringanimation/init(perceptualduration:bounce:)>
- Settling and perceptual duration are different things. Settling is "a bit unpredictable",
  so completion should key off the perceptual duration (WWDC23).

### WWDC18 "Designing Fluid Interfaces" (<https://developer.apple.com/videos/play/wwdc2018/803/>)

The most useful public statement of Apple's motion philosophy.

- **Response:** "Everything needs to respond instantly." Avoid timers and dwell delays. The
  multitasking pause is detected from a spike in finger deceleration, not from a timeout.
  A double-tap recogniser delays single taps by about half a second, so avoid stacking them.
- **Redirection:** every transition can be grabbed and reversed mid-flight ("a fully
  redirectable interface").
- **Momentum projection:** project the release velocity with the scroll view's deceleration
  rate, then retarget to the nearest valid endpoint. The talk does not show the formula. The
  form commonly attributed to its sample code is `projected = position + v · r / (1 − r)`,
  with `v` in points per ms and `r` the per-ms rate. Unverified, because the sample was not
  available. <https://mcpservers.org/agent-skills/emilkowalski/apple-design>
- **Springs:**
  - Use damping and response, never duration ("Don't use duration when we're describing
    elastic behaviors").
  - Start at 100 % damping.
  - Add overshoot only when the driving gesture had momentum. Their example is 80 % damping
    for swipe-to-dismiss and 100 % for tap-to-expand. "If a gesture has momentum, and there
    isn't any overshoot, it can often feel broken."
- **Rubber banding:** resistance grows past the edge, and the content keeps tracking. A hard
  stop "wouldn't know the difference between a frozen phone and … the edge".
- **Hysteresis:** about 10 pt of movement before a drag starts (iOS).
- **Spatial consistency:** an object leaves the way it came. Hinting: transitions grow from
  the source, and a module grows toward the finger before it opens.
- **Frame content:** "it's not just about framerate. It's what's in the frames." Fast motion
  at a low rate strobes; motion stretching is used on app open.

### Curves and defaults (AppKit and Core Animation)

- `CAMediaTiming.duration` defaults to 0 (the implicit 0.25 s comes from the transaction), per
  <https://developer.apple.com/documentation/quartzcore/camediatiming/duration>. The
  `NSAnimationContext` and `CATransaction` default of 0.25 s is unverified.
- `NSWindow.animationResizeTime` reads `NSWindowResizeTime`. The default is 0.20 s per 150 px
  of frame change, "may be different in different releases".
  <https://developer.apple.com/documentation/appkit/nswindow/animationresizetime(_:)>
  This is the one documented system window timing, and it is distance-dependent, as a spring is.
- `NSWorkspace.accessibilityDisplayShouldReduceMotion`: "avoid large animations, especially
  those that simulate the third dimension".
  <https://developer.apple.com/documentation/appkit/nsworkspace/accessibilitydisplayshouldreducemotion>
- HIG Motion (<https://developer.apple.com/design/human-interface-guidelines/motion>):
  - "Add motion purposefully".
  - "Make motion optional".
  - "Aim for brevity and precision in feedback animations".
  - "Generally avoid adding motion to UI interactions that occur frequently".
  - "Let people cancel motion … don't make people wait for an animation to complete".
  - Liquid Glass motion is "more subdued" for trackpad input than for direct touch.

### Unpublished macOS timings

Apple publishes no number for any of these. The values are our proposals, reasoned from the
principles above and from what other systems use.

| Animation | macOS behaviour (observed, unverified) | Proposal for Bande |
|---|---|---|
| Menu selection | The chosen item blinks once, then the menu fades | Blink one on-off cycle at about 60 ms, then fade 150 ms. The action fires at once and does not wait for the fade |
| Menu dismiss without choice | Fades out | 120 ms opacity fade |
| Sheet | Slides down out of the title bar | `standard` spring on translate-y and clip; dims the parent |
| Popover | Fades and scales slightly from its anchor | `quick` spring, scale 0.95 to 1 from the arrow tip, plus opacity |
| Dock launch bounce | Bounces until the app is up | DESIGN uses a gentle pulse instead (taskbar.md). Keep that; no bounce |
| Dock magnification | Icons scale with pointer distance, tracking 1:1 | Direct mapping, no spring while hovering; spring back with `quick` on leave |
| Spotlight / launcher | Appears instantly, fades | 100 ms fade plus 0.98 to 1 scale. Text input is live from frame 1 |
| Notification banner | Slides in from the right edge, stacks | `standard` slide plus fade; the stack reflows with `standard`; dismiss by flick uses `momentum` |
| Focus ring | Animates from larger to the control's bounds | `quick` spring on inset from +6 px to 0, plus opacity |
| Overlay scrollbars | Appear while scrolling, fade after idle | Show on scroll; fade after 1.0 s idle over 200 ms. Unverified macOS value |
| List insert / delete | `NSTableView` slide and fade options | Rows slide with `standard`; the deleted row fades with `quick` |
| Quick Look | Zooms from the file icon and back | `standard` from the icon's rect; closes back into it, or fades if the icon is gone |
| App launch | Window zooms from the Dock icon (older macOS), now a plain fade-in (unverified) | Window opens with scale 0.95 to 1 plus fade (windows.md). The Dock icon is the origin only for minimise |
| Wrong password | Field shakes horizontally | About three visible swings, ±10 px, about 400 ms. Implement as the `shake` spring with an initial velocity kick, not as keyframes |
| Window resize by zoom | 0.2 s per 150 px (documented above) | `standard` spring on the frame (distance-scaled for free) |

## What others publish

| System | Values |
|---|---|
| GNOME Shell `windowManager.js` | Window map 150 ms ease-out-expo; destroy 150 ms ease-out-quad; dialogs 100 ms; minimise 400 ms ease-out-expo; tile preview and size change 250 ms; dim 500 / undim 250 ms. <https://gitlab.gnome.org/GNOME/gnome-shell/-/raw/main/js/ui/windowManager.js> |
| libadwaita | `AdwSpringAnimation`: damping ratio, mass, stiffness, epsilon, `clamp`, `initial-velocity`, estimated duration. <https://gnome.pages.gitlab.gnome.org/libadwaita/doc/main/class.SpringAnimation.html> |
| niri defaults | workspace-switch: spring ζ 1.0, k 1000. View, window movement, resize and overview: ζ 1.0, k 800, epsilon 0.0001. Window open 150 ms ease-out-expo; close 150 ms ease-out-quad. Notifications: ζ 0.6, k 1000. <https://github.com/YaLTeR/niri/wiki/Configuration:-Animations> |
| KWin | Magic Lamp 250 ms; Squash 250 ms; Scale (open/close) 200 ms; Glide 160 ms; all scaled by the global speed factor. <https://raw.githubusercontent.com/KDE/kwin/master/src/plugins/scale/package/contents/code/main.js>, <https://raw.githubusercontent.com/KDE/kwin/master/src/plugins/glide/glide.cpp> |
| Material 3 | Springs as fast / default / slow × spatial / effects. Spatial fast: ζ 0.9, k 1400. Effects fast: ζ 1, k 3800. <https://raw.githubusercontent.com/material-components/material-components-android/master/docs/theming/Motion.md>; numbers seen only in third-party packages, unverified |

Reading: niri's k 800 to 1000 (mass 1) is ω 28 to 32, a perceptual duration of about 0.2 s.
That is faster than Apple's 0.5 s default but right for compositor-level moves, where the
user is waiting. Our presets sit between the two.

## Reduce Motion, Reduce Transparency, Increase Contrast

- **macOS:** Reduce Motion replaces zooms and slides (Spaces, Mission Control, app open) with
  cross-fades. Reduce Transparency makes materials opaque. Increase Contrast adds borders and
  darkens edges, and implies Reduce Transparency.
  <https://support.apple.com/guide/mac-help/change-display-settings-for-accessibility-unac089/mac>
  (page not re-fetched in this pass)
- **Linux:** `org.freedesktop.appearance` in the Settings portal has gained `reduced-motion`
  (0 or 1) next to `color-scheme`, `accent-color` and `contrast` (xdg-desktop-portal 1.21).
  <https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.Settings.html>,
  <https://phoronix.com/news/xdg-desktop-portal-1.21>
- GTK 4.22 reads it as `gtk-interface-reduced-motion`.
  <https://docs.gtk.org/gtk4/property.Settings.gtk-interface-reduced-motion.html>
- No portal key exists for reduced transparency (unverified).

## For Bande

1. **`bande-spring` presets (now, `crates/libs/spring`).**
   - Add `Spring::from_duration_bounce(duration, bounce)` (ω = 2π/d, ζ = 1 − b) and named
     constants. Keep the API in `ω` and `ζ`; designers speak duration and bounce.
   - Add `Animation::with_velocity` / `retarget_with_velocity` so gesture release can inject
     velocity. `State` already carries velocity.
   - Add a decay (`Decay { rate_per_ms }`) with `project(position, velocity)` for momentum.
     It belongs in the same crate, because every gesture end uses it.
2. **Vector animation (now).**
   - Window frames animate x, y, w, h and opacity together. Make `Animation` generic over a
     small `Animatable` (f64, `[f64; 2]`, `[f64; 4]`), or keep one `Animation` per channel
     sharing a spring.
   - Per-channel is simpler; retargeting must happen on all channels in the same frame.
3. **One clock (now).**
   - Compositor animations sample at the frame's predicted presentation time from
     `crates/platform/compositor/src/clock.rs`, not at render time.
   - The toolkit samples at the time of the `wl_surface.frame` callback.
   - This is what makes 120 Hz look smooth.
4. **Reduce Motion (now).**
   - The compositor and toolkit read `reduced-motion` from the portal (the toolkit already
     reads `org.freedesktop.appearance` in `crates/toolkit/platform/src/appearance.rs`).
   - `xdg-desktop-portal-bande` serves it from `config.kdl` (`appearance { reduce-motion }`).
   - With it on: spatial animations become a `fade` cross-fade, and gestures still track 1:1
     (content following the fingers is not "motion" to reduce) but release with `fade`.
5. **Rule for overshoot.**
   - Bounce > 0 only when the driving input had momentum (fling, flick-dismiss), via
     `momentum`.
   - Taps, keys and clicks use critically damped presets. This is Apple's WWDC18 rule and keeps
     the desktop calm.
6. **Never gate input on animation.** The compositor sends configure, focus and keyboard to the
   destination window at the start of an animation (DESIGN §3, "Fast first").

## Motion presets for `bande-spring`

Mass 1, `k = ω²`. "Settles" is the time to within 0.1 % of the distance, computed from the
closed form. The perceptual duration is what the eye reads as done.

The durations are deliberately shorter than SwiftUI's app defaults (0.5 s `smooth`): a window
manager is something the user waits on, so it must feel fast, and even `standard` still gives
about 30 frames of motion at 120 Hz (15 at 60 Hz), which reads as smooth rather than a jump.
They sit between macOS' observed system speeds (0.2 to 0.3 s for window zoom, Mission Control
and Spaces) and niri's 0.2 s springs.

An animation stops redrawing once the change is no longer visible, not at a unit-less epsilon:
`Animation::is_at_rest(precision)` holds when the value is within `precision` of its target and
its speed would move it less than `precision` in 20 ms. The compositor uses 0.1 *physical*
pixel for geometry (divided by the output scale for logical values, and by the output's size
for progress values such as the workspace view or the overview) and 1/512 for opacity. A
`standard` window open at 120 Hz therefore renders 37 frames instead of the 61 the earlier
0.25 px / 0.35 s combination needed.

| Name | Use | Duration, bounce | ω (rad/s), ζ | k | Overshoot | Settles |
|---|---|---|---|---|---|---|
| `interactive` | Following a pointer or finger when not 1:1 (rubber-band tracking, Dock magnification release, slider thumb catch-up) | response 0.15, ζ 0.86 (SwiftUI `interactiveSpring`) | 41.9, 0.86 | 1755 | 0.5 % | 0.19 s |
| `quick` | Small UI: popover, menu appear, focus ring, switcher, tile outline, hover states, button press, window close | 0.18 s, 0 | 34.9, 1.0 | 1218 | none | 0.26 s |
| `standard` | Default for windows: open, zoom, full screen, tile, workspace switch, overview, sheet, Quick Look, notification slide | 0.25 s, 0 | 25.1, 1.0 | 632 | none | 0.37 s |
| `gentle` | Large or long-distance moves: minimise to and restore from the bar, wallpaper cross-fade | 0.35 s, 0 | 18.0, 1.0 | 322 | none | 0.51 s |
| `momentum` | Release of a gesture with velocity: flick-dismiss a notification, fling a window to a tile, workspace swipe with fast release | 0.3 s, 0.15 (SwiftUI `snappy` bounce) | 20.9, 0.85 | 439 | 0.6 % | 0.39 s |
| `bouncy` | Attention only: a bar badge appearing, a refused action's nudge | 0.4 s, 0.3 (SwiftUI `bouncy`) | 15.7, 0.70 | 247 | 4.6 % | 0.63 s |
| `shake` | Wrong password: target 0, initial velocity about 600 px/s | period 0.12 s, ζ 0.2 | 52.4, 0.2 | 2742 | n/a (oscillates) | 0.4 s visible |
| `fade` | Opacity-only changes (hide, HUD fade-out, hover), and the Reduce Motion substitute for every spatial preset | 0.12 s, 0 | 52.4, 1.0 | 2742 | none | 0.17 s |
| `decay` | Scroll and swipe momentum after release | per-ms rate 0.998 (normal) or 0.99 (fast), UIKit values (unverified exact) | n/a | n/a | n/a | velocity-dependent |
| `rubber-band` | Overscroll resistance while dragging | `d · c · x / (d + c · x)`, c = 0.55 (unverified UIKit constant), then `standard` back to the edge on release | n/a | n/a | n/a | n/a |

Priority: `quick`, `standard`, `gentle`, `fade` and `decay` now (they are DESIGN's quick,
standard and gentle plus what gestures need); `interactive`, `momentum` and `rubber-band` with
the first gesture work; `bouncy` and `shake` later.
