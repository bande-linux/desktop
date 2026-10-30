# Research: how macOS feels in motion and how its parts connect

Research for Unbound's developers. macOS is the quality bar here, not the blueprint; each file
ends with a **For Unbound** section saying what to build, where, and when. Sources are linked
inline in each file, and "unverified" marks what we could not check against a primary source.
Apple publishes almost no system animation timings, so where numbers are missing we propose
values and give the reasoning.

| File | Topic |
|---|---|
| [animations.md](animations.md) | Apple's spring model and presets, WWDC18 principles, unpublished macOS timings with proposals, other desktops' values, Reduce Motion, **the preset table for `unbound-spring`** |
| [gestures.md](gestures.md) | Trackpad gestures, 1:1 tracking, velocity hand-off, rubber band, kinetic scrolling, libinput and `zwp_pointer_gestures_v1` |
| [drag-and-drop.md](drag-and-drop.md) | Where DnD works on macOS, visuals, modifiers, spring-loading, `wl_data_device` actions, `xdg-toplevel-drag-v1` |
| [window-management.md](window-management.md) | Tiling, zoom, full screen, minimise, hide, switcher, Mission Control, Snap Layouts, niri and KWin |
| [integrations.md](integrations.md) | Quick Look, proxy icons, clipboard, services, text system, keyboard reach, accessibility switches, launcher as hub |

## What makes macOS feel fluid: 14 principles

1. **Nothing waits for an animation.** Input reaches the destination on the next frame, and
   motion runs alongside. Apple: "Everything needs to respond instantly" and "don't make people
   wait for an animation to complete" (WWDC18 803; HIG Motion). Concretely, the compositor sends
   configure, focus and keyboard at the start of a transition, never at its end.
2. **Every animation is a spring, retargeted, never restarted.** SwiftUI's `spring` replaces a
   running spring "preserving velocity". Our crate already keeps position and velocity
   continuous on retarget. Use it for *every* state change, so double-pressing a shortcut
   reverses smoothly.
3. **Think in duration and bounce; build in ω and ζ.** Apple's model is ω = 2π / duration and
   ζ = 1 − bounce. Default bounce is 0. Add bounce (0.15) **only when the driving gesture had
   momentum**: Apple's own rule, and the one that keeps a desktop calm.
4. **Few presets, used everywhere.** The table below has seven springs plus decay and
   rubber-band. A developer picks by role, not by number.
5. **Gestures track 1:1, then hand velocity to the spring.** Progress follows the fingers
   exactly, reversal is free, and release projects the velocity with a deceleration of about
   0.997 per ms to pick the target. The spring then starts at the finger's speed: no jump, no
   restart. A new touch during the settle grabs the animation where it is.
6. **Edges are soft.** Past the end of anything (scroll, workspace row, zoom limit), content
   keeps moving with growing resistance (`d·c·x/(d+c·x)`, c = 0.55) and springs back. A hard
   stop reads as a frozen machine (WWDC18).
7. **Inertia is a system service.** macOS generates scroll momentum centrally, so every app
   decays identically. On Wayland, libinput does not do it. Our toolkit must implement it once
   from `axis_source` and `axis_stop`, and every app gets the same curve.
8. **Every window transition has a visible source and destination.** Minimise goes into the
   exact Dock icon, Quick Look grows from the file icon, the overview flies windows from their
   real positions, and an object leaves the way it came. Nothing appears from nowhere.
9. **One object, one identity.** The window in the switcher, in the overview and on the desktop
   is the same live texture, scaled. Never a different-looking thumbnail.
10. **Drag and drop works between everything, and says what it will do.** Every visible thing
    is a source or target: Dock icons, Trash, Terminal, the title bar, the screenshot thumbnail.
    The cursor and a short label tell copy from move; a refused drop slides back home.
11. **Hover to open during a drag (spring-loading).** Folders, Dock icons, tabs and workspace
    thumbnails open after a dwell (we propose 700 ms; Space opens at once), so a drag can travel
    anywhere without letting go.
12. **Use dwell and hysteresis to avoid accidents, never as decoration.** A 150 ms rest before a
    tile preview, 150 ms before the switcher overlay, and a few px before a drag starts.
    Otherwise there are no timers: WWDC18 replaces dwell timers with motion analysis wherever it
    can.
13. **One shared system per concern.** Quick Look, the text system (spelling, look up,
    substitutions), the clipboard and Reduce Motion are each one service every app gets for
    free. Unbound's equivalents live in the toolkit and session services, never per app.
14. **Reduce Motion swaps space for opacity; it does not remove feedback.** Slides and zooms
    become cross-fades, and gestures still track the fingers. Read `reduced-motion` from the
    `org.freedesktop.appearance` portal (xdg-desktop-portal 1.21).

## Motion presets for `unbound-spring`

Mass 1, k = ω². "Settles" is the time to within 0.1 % of the distance. Details and reasoning
are in [animations.md](animations.md#motion-presets-for-unbound-spring).

| Name | Use | Parameters | Feel |
|---|---|---|---|
| `interactive` | Following input when not 1:1 (catch-up, rubber-band tracking, magnification release) | response 0.15 s, ζ 0.86 (ω 41.9, k 1755) | Immediate; settles 0.19 s |
| `quick` | Popovers, menus, focus ring, switcher selection, tile outline, window close | 0.18 s, bounce 0 (ω 34.9, ζ 1, k 1218) | Crisp; settles 0.26 s |
| `standard` | Window open, zoom, full screen, tile, workspace switch, overview, sheets, Quick Look, banners | 0.25 s, bounce 0 (ω 25.1, ζ 1, k 632) | The default; settles 0.37 s |
| `gentle` | Minimise and restore, long moves | 0.35 s, bounce 0 (ω 18.0, ζ 1, k 322) | Calm; settles 0.51 s |
| `momentum` | Release of a flick: dismiss, fling to a tile, fast workspace swipe | 0.3 s, bounce 0.15 (ω 20.9, ζ 0.85, k 439) | 0.6 % overshoot; settles 0.39 s |
| `bouncy` | Attention only | 0.4 s, bounce 0.3 (ω 15.7, ζ 0.7, k 247) | 4.6 % overshoot; use rarely |
| `shake` | Wrong password | period 0.12 s, ζ 0.2, initial velocity kick | About three swings in 0.4 s |
| `fade` | Opacity, and Reduce Motion's substitute for every spatial preset | 0.12 s, bounce 0 (ω 52.4, ζ 1, k 2742) | Settles 0.17 s |
| `decay` | Scroll and swipe momentum | 0.998 per ms (normal), 0.99 (fast) | Velocity-dependent |
| `rubber-band` | Overscroll resistance | `d·c·x/(d+c·x)`, c = 0.55; back with `standard` | Soft edge |

## Biggest gaps found in the code (2026-10-29)

- **Gestures.** `crates/platform/compositor/src/input.rs` drops every libinput gesture event,
  and no `zwp_pointer_gestures_v1` global exists, although `grab.rs` already forwards
  `gesture_*`. Nothing in catalogue §7 can start until this is in.
- **Scrolling.** `crates/toolkit/platform/src/seat.rs` reads `wl_pointer.axis` only. Without
  `axis_source` and `axis_stop` there can be no kinetic scrolling and no rubber band, and there
  is no scroll view widget yet.
- **Springs.** `unbound-spring` has no named presets, no duration and bounce constructor, no
  velocity injection on retarget and no decay. Nothing outside the crate uses it yet.
- **Drag and drop.** The DnD grab handlers are Smithay defaults. We found no drag-icon
  rendering, no modifier-to-action mapping (DESIGN's Alt-copy) and no slide-back.
  `xdg-toplevel-drag-v1` is not in Smithay.
- **Shell to compositor geometry.** Minimise into the icon needs the Dock to tell the
  compositor where each icon is. `unbound_shell_v1` needs a `set_icon_rect` request.
