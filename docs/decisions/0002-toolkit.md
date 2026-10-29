# UI Toolkit: Decision Record

Status: **accepted** (2026-10-29, Option A: extend `unbound-toolkit`). Scope: which widget
layer Unbound's own apps and shell (panel, menu bar, launcher, Files, Settings, Terminal,
lock screen, greeter, Control Center) are built on, and in which order it grows to AppKit
depth. Rendering (Vello, Parley) is settled by [`0001-rendering.md`](0001-rendering.md) Decision 2 and is not
reopened here.

## Context

What the apps need, at macOS quality: a full pointer model (every button, click count,
modifiers, hover, enter and leave, precise scrolling with phases and momentum, cursor
shapes); a retained widget tree with hit testing and a focus chain; interruptible spring
animation of properties; menus and context menus as `xdg_popup`s (submenus, shortcuts,
checkmarks, type-select, keyboard navigation); scroll views with elastic overscroll;
list, table, outline and icon-grid views on one selection model with rubber-band
selection; popovers, sheets, alerts, segmented controls, pop-up buttons, search fields,
sidebars, toolbars, split views, tab views, title bars; drag and drop; tooltips;
AccessKit on AT-SPI; IME; fractional scale.

What `crates/toolkit/toolkit` has today (about 8,300 lines with tests):
- **Kept regardless of the decision:** `Window` on libwayland with `xdg_toplevel`,
  `zwlr_layer_surface_v1` and session lock; first frame on `vello_cpu` into `wl_shm`,
  then `vello_gpu`; fractional scale and viewporter; text-input v3; clipboard; AccessKit
  published on AT-SPI; the `Scene` API with a text pipeline tuned past Vello's defaults
  (0.004 px flattening, unhinted `skrifa` outlines, Inter/Inter Display switch, baselines
  on whole pixels); tokens; the `unbound-spring` crate (analytic damped spring with
  `retarget`, not yet used by any crate).
- **Thin:** `ui` has `Stack`, `Label`, `Button`, `Toggle`, `Slider`, `TextField`,
  `SecureField` and a WIP `TitleBar`. `ui::Event` is `PointerDown/Move/Up(Point)` with no
  button, count or modifiers; `seat.rs` handles only `BTN_LEFT`, binds `wl_seat` at
  version 7 or lower (so no `axis_value120`, which needs 8 [15]) and ignores
  `axis_source` and `axis_stop`, so finger scrolling cannot be told from a wheel and has
  no end to start momentum from. Scroll arrives as `WindowEvent::Scroll { dx, dy }`
  outside the widget tree. There is no hover, enter or leave per widget, no cursor
  setting, and a `Window` owns exactly one surface, so no popups. `Ui` re-measures the
  whole tree each frame and finds widgets by recursive search.
- **Bypassed:** Files and Settings hand-paint `Scene`s in `view.rs` (353 and 449 lines)
  with their own hit testing; the greeter and terminal use no widgets; panel, launcher,
  Control Center and lock screen use `Ui` for a few controls only.
- The compositor already serves `xdg_popup` (including `get_popup` from layer
  surfaces), `wp_cursor_shape_v1`, `wl_data_device` and text-input v3, so every
  protocol the toolkit needs exists on the other side.
- Pinned in `Cargo.toml`: `vello_cpu`/`vello_gpu` 0.3.0, `parley`/`fontique` 0.12.0,
  `accesskit` 0.25.2, `accesskit_unix` 0.24.1, toolchain 1.99.1.

### Masonry today (verified 2026-10-29 unless marked as inference)

- Masonry describes itself as "a foundational framework for building GUI libraries" that
  owns the widget tree and handles focus, pointer interactions and accessibility
  centrally, on Vello, Parley and AccessKit [1]. Xilem's README calls the project "an
  experimental Rust architecture for reactive UI" [12].
- Releases: 0.4.0 on 2025-10-29, 0.3.0 on 2025-05-10, 0.2.0 on 2024-05-07 [2]. No
  release in the 12 months since; `main` is active (commits on 2026-10-27 and 10-29).
- `main` pins `vello` 0.8 (the compute renderer, not sparse strips), `parley` 0.8,
  `accesskit` 0.24, `wgpu` 28, `winit` 0.30 [3]. Painting goes through the `imaging`
  0.0.1 abstraction; its `vello_cpu` backend depends on `vello_cpu` 0.0.7 [11]. Ours are
  `vello_cpu` 0.3, `parley` 0.12, `accesskit` 0.25.2: every shared crate is behind.
- Host interface: `RenderRoot` takes `handle_pointer_event`, `handle_text_event`,
  `handle_window_event`, `handle_access_event`, returns a `VisualLayerPlan` and a
  `TreeUpdate` from `redraw()`, and emits `RenderRootSignal`s (`SetCursor`, `StartIme`,
  `ImeMoved`, `RequestAnimFrame`, `NewLayer`, `ClipboardStore`, `DragWindow`) [4][9].
  A host other than `masonry_winit` is therefore possible; `masonry_winit` is the only
  desktop host shipped [1].
- Input vocabulary is `ui-events` 0.3: `Down/Up` with button, `Move`, `Enter`, `Leave`,
  `Cancel`, `Scroll`, `Gesture`; `PointerState` carries buttons, modifiers and click
  `count`. `PointerScrollEvent` has a delta and no phase or momentum field [10]. Widgets
  get `HoveredChanged`, `ActiveChanged`, `FocusChanged` updates and `Ime` events [5].
- Widgets on `main`: button, checkbox, radio, switch, slider, step input, selector,
  text input/area, prose, label, flex, grid, split, portal (scroll), virtual scroll,
  collapse panel, disclosure, progress, spinner, image, svg, zstack and a few
  containers [6]. No menu, context menu, table, outline, icon grid, selection model,
  toolbar, tab view, popover, sheet, alert, or drag and drop. Layers exist for tooltips
  and the selector's list and are drawn inside the window; menus are an open tracking
  issue [7], native child windows an open draft PR (#1442).
- Scrolling: the maintainers call `Portal` "some of the oldest and most cursed code in
  Masonry" and list its rewrite as open work [8]; it has no momentum or elastic overscroll.
- Animation: a widget gets `on_anim_frame(duration)` and calls `request_anim_frame` [5];
  there is no property-animation or spring system.

## Comparison

| Need | A. Extend `unbound-toolkit` | B. Masonry under our `Window` | C. libcosmic / Slint / GTK4 |
| :--- | :--- | :--- | :--- |
| Renders through our `Scene`, first frame on CPU | yes, unchanged | needs an `imaging` sink to our `Scene` and a bridge from Parley 0.8 glyph runs to our text path | no: own renderers and text stacks |
| Vello/Parley/AccessKit versions | ours, bumped when we choose | lock-step with Masonry, which trails by 3–4 minor versions; a fork to keep ours | not applicable |
| Pointer model | to build (about 1.5 weeks) | `ui-events` covers buttons, count, enter/leave; scroll phases still ours | GTK4 complete; others partial |
| Retained tree, hit test, focus, hover, capture, IME, a11y per widget | partly there; core rewrite about 3 weeks | present and tested | present |
| Spring animation, momentum, elastic scroll | to build on `unbound-spring` | to build, inside widgets we do not own | GTK4 kinetic scroll, own curves |
| Menus as `xdg_popup`, popovers, tooltips | to build (multi-surface `Window`) | layers are in-window; mapping `NewLayer` to popups is ours | GTK4 yes; others partial |
| Tables, outline, icon grid, selection, rubber band, DnD, toolbars, tabs, sheets | to build | to build (only `VirtualScroll` helps) | GTK4 yes |
| Layer-shell surfaces | yes, today | through our host | libcosmic via its Wayland backend; Slint no built-in support [14]; GTK4 via `gtk4-layer-shell` [17] |
| Control over feel and look | complete | through Masonry's property system and upstream review | low (theming within their model) |
| API stability | ours | experimental, one release a year, breaking changes expected [1][12] | stable (GTK4), moving (libcosmic) |
| Rough effort to the feature list | about 25 engineer-weeks | about 30, plus a recurring merge cost | a rewrite of the toolkit and Decision 2 |

## Recommendation

**Option A: extend `unbound-toolkit`**, taking Masonry's architecture as a reference
rather than as a dependency.

1. **Masonry does not have what is expensive.** Of the feature list, Masonry saves the
   widget core: tree, passes, hover, capture, focus, IME routing, accessibility per
   widget, a test harness. That is perhaps 4–5 of the 25 weeks. The bulk (menus as
   popups, collection views with a selection model, momentum and elastic scrolling,
   springs, drag and drop, toolbars, sheets, AppKit-sized controls) is missing upstream
   and would be built either way, inside a framework whose own maintainers flag its
   scroll widget for a rewrite [8] and whose menus are a tracking issue [7].
2. **It would undo the stack we just chose.** Masonry pins Parley 0.8, AccessKit 0.24
   and paints through `imaging`, whose CPU backend uses `vello_cpu` 0.0.7 [3][11].
   Adopting it means downgrading Parley, Vello and AccessKit, or maintaining a fork that
   tracks ours; and the text quality work in our `Scene` (0.004 px flattening, unhinted
   outlines, Inter Display switch, whole-pixel baselines) would sit behind a bridge or be
   lost. Two Parley/fontique copies in one process would also load the font collection
   twice, which works against the 30 MB RSS budget (inference).
3. **Feel is the product.** Interruptible springs, momentum that matches macOS decay,
   rubber-band overscroll and menu tracking are where macOS smoothness lives. Owning the
   event loop, frame clock and every widget keeps those decisions in one codebase with
   our tests and perf gates, rather than in upstream review of an experimental API that
   has made one release in a year [2].

Option C does not fit: libcosmic (iced) and Slint bring their own renderers and text
stacks, which would reverse Decision 2, and Slint has no built-in layer-shell [14].
libcosmic's accessibility is marked experimental [13]. GTK4 via gtk-rs is the
most complete, but it is a C runtime with its own renderer and CSS theming, which
means an own look fights the toolkit, and our Vello/Parley/AccessKit work is discarded.

## Consequences

Order of work; each step lands with tests in `crates/toolkit/toolkit/tests` and keeps the
exact perf metrics green.

1. **Input model.** Replace `ui::Event` pointer variants with a pointer event shaped
   like `ui-events` [10]: `Down`/`Up { button, count, modifiers, position }`, `Move`,
   `Enter`, `Leave`, `Cancel`, and `Scroll { delta, unit: Line|Pixel, source:
   Wheel|Finger|Continuous, phase: Began|Changed|Ended|Momentum }`. `seat.rs` binds
   `wl_seat` up to version 9, reads every button, `axis_source`, `axis_stop`,
   `axis_value120` and `axis_relative_direction` [15], derives click count from the
   double-click interval and distance, and sets cursors through `wp_cursor_shape_v1` [16].
   `WindowEvent::Scroll` folds into the pointer event.
2. **Widget core.** A retained tree with parent links and per-widget flags (hovered,
   active, focused, disabled, needs layout, needs paint); dispatch with pointer capture
   and bubbling; hover and enter/leave from the hit path; a focus chain with full
   keyboard access; cached layout invalidated by flags; per-widget damage feeding
   `Window` damage. Masonry's passes (event, update, layout, compose, paint, access) are
   the reference design [4][5].
3. **Animation.** A property animator for `f32`, points, rects and colours on
   `unbound-spring`, driven by `wl_surface.frame` with presentation time, interruptible
   by `retarget` with the current velocity, and honouring a reduce-motion setting.
4. **Popups.** `Window` owns several surfaces: `xdg_popup` with positioner, grab and
   `reposition` from toplevels, and `get_popup` from layer surfaces [17]. Then menus
   (submenus, shortcuts, checkmarks, type-select, keyboard navigation), context menus,
   tooltips, popovers and pop-up buttons on top.
5. **Scroll view.** Momentum synthesized after `axis_stop` for finger scrolling, elastic
   overscroll on a spring, overlay scrollers, accessibility scroll actions.
6. **Collection views.** One selection model (single, extend, toggle, rubber band,
   type-select) under list, table, outline and icon grid, virtualized.
7. **Remaining controls and containers**: segmented control, search field, sidebar,
   toolbar, split view, tab view, sheets, alerts, title bar finished; then drag and drop
   on `wl_data_device`.
8. **Port the apps.** Files and Settings move from hand-painted `Scene`s to widgets as
   steps 5–6 land; the shell clients follow with step 4. [`docs/architecture/toolkit.md`](../architecture/toolkit.md) and
   [`docs/parity.md`](../parity.md) are updated as each step lands.

## Risks and how to contain them

- **Underestimating the widget core.** Masonry's history shows the core is subtle
  (capture, stashed children, layout invalidation; its fixes in October 2026 are in
  exactly these areas). Contain: read Masonry's passes and tests before step 2, port
  their tricky cases as tests, and keep the core small enough to reason about.
- **Effort.** 25 engineer-weeks is an estimate with wide error. Contain: the order
  above delivers usable value at each step (input, then menus, then scroll); widgets are
  built when an app needs them, not ahead of it.
- **Feel regressions.** Contain: virtual-clock tests for springs and momentum against
  recorded macOS curves, golden images for every control in light and dark, and the
  existing exact metrics for repainted pixels.
- **Missing a better upstream.** If Masonry stabilises and moves to our Vello/Parley
  versions, adopting its core later stays possible because our input vocabulary mirrors
  `ui-events`. Revisit at Masonry's next release.
- **Budgets.** More widgets and animation state must keep app start ≤ 100 ms and
  ≤ 30 MB RSS on the CPU path. Contain: `unbound-sample` grows to exercise the new core
  and stays under the existing gate.

## Sources

1. Masonry 0.4.0 crate documentation: <https://docs.rs/masonry/latest/masonry/>
2. Masonry versions on crates.io: <https://crates.io/crates/masonry/versions>
3. Xilem workspace `Cargo.toml` (pinned versions): <https://github.com/linebender/xilem/blob/main/Cargo.toml>
4. `RenderRoot` and `RenderRootSignal`: <https://github.com/linebender/xilem/blob/main/masonry_core/src/app/render_root.rs>
5. Masonry events (`Update`, `Ime`, `WindowEvent`): <https://github.com/linebender/xilem/blob/main/masonry_core/src/core/events.rs>
6. Masonry widgets on `main`: <https://github.com/linebender/xilem/tree/main/masonry/src/widgets>
7. Tracking issue for layers and menus: <https://github.com/linebender/xilem/issues/1343>
8. Tracking issue for Portal/Scroll widget: <https://github.com/linebender/xilem/issues/1344>
9. `VisualLayerPlan`: <https://github.com/linebender/xilem/blob/main/masonry_core/src/app/visual_layers.rs>
10. `ui-events` pointer types: <https://github.com/endoli/ui-events/blob/main/ui-events/src/pointer/mod.rs>
11. `imaging_vello_cpu` dependencies: <https://crates.io/crates/imaging_vello_cpu>
12. Xilem README: <https://github.com/linebender/xilem/blob/main/README.md>
13. libcosmic: <https://github.com/pop-os/libcosmic>
14. Slint discussion on `zwlr_layer_shell_v1`: <https://github.com/slint-ui/slint/discussions/1765>
15. `wl_pointer` (axis source, stop, value120, relative direction): <https://wayland.app/protocols/wayland#wl_pointer>
16. Cursor shape protocol: <https://wayland.app/protocols/cursor-shape-v1>
17. `xdg_popup` and layer-shell `get_popup`: <https://wayland.app/protocols/xdg-shell#xdg_popup>, <https://wayland.app/protocols/wlr-layer-shell-unstable-v1>; `gtk4-layer-shell`: <https://github.com/wmww/gtk4-layer-shell>
