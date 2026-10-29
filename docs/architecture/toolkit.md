# Toolkit

Every Unbound shell client and app draws with one shared toolkit, `unbound-toolkit`, so that
look, shortcuts and accessibility are the same everywhere. It draws with Vello and lays out text
with Parley ([`../decisions/0001-rendering.md`](../decisions/0001-rendering.md), Decision 2).
How it grows to AppKit depth is decided in
[`../decisions/0002-toolkit.md`](../decisions/0002-toolkit.md).

## Crates

- `unbound-graphics` (`crates/toolkit/graphics`): scene, Vello CPU and GPU renderers, Parley
  text, design tokens, and the input event types. No Wayland.
- `unbound-widgets` (`crates/toolkit/widgets`): the widget tree and widgets, on graphics only,
  so widget tests need no compositor.
- `unbound-toolkit-platform` (`crates/toolkit/platform`): Wayland windows and roles, seat input,
  clipboard, text input, capture, the run loop, accessibility publication and appearance.
- `unbound-toolkit` (`crates/toolkit/toolkit`): the facade apps depend on; `ui` is
  `unbound-widgets`.

CI fails when a service depends on the widgets, platform or facade, or when graphics or widgets
depend on Wayland.

## Appearance

Every window follows the system appearance itself: the platform reads `color-scheme` and
`accent-color` of `org.freedesktop.appearance` from the XDG Settings portal, at window creation
and live on `SettingChanged`, and delivers `WindowEvent::AppearanceChanged`. Apps never read
`config.kdl` for it.

## Goals

- **Semantic colour tokens**: colour roles (`surfaceBackground`, `surfaceElevated`,
  `labelPrimary`, `labelMuted`, `accentColor`, `separatorColor`) for light and dark, switched
  live when the appearance changes.
- **Accessibility built in**: every widget produces and maintains an AccessKit node, published
  on AT-SPI, so screen readers work by construction.
- **Standard controls**: scroll views with spring physics, text fields, sliders, switches and
  tab views with the same keyboard and pointer behaviour in every app.

## Implemented (`crates/toolkit/toolkit`)

- **Tokens.** The six colour roles for light and dark schemes, each label meeting WCAG AA
  contrast (4.5:1) and the accent 3:1 on both surfaces; a spacing scale of 2, 4, 8, 12, 16, 24
  and 32 px.
- **Scene.** A `Scene` of fills, strokes, linear gradients, rounded rectangles, clips, shadows
  and text that `CpuRenderer` (`vello_cpu`) and `GpuRenderer` (`vello_gpu` on Vulkan) draw from
  one encoder.
  - Curves are flattened by the toolkit at 0.004 px instead of Vello's 0.25 px, so edges are
    within 1/255 of their exact area coverage. Glyph outlines come from `skrifa`, unhinted and in
    font units (no 26.6 rounding), are cached per glyph and size, and are drawn at fractional
    pen positions with grayscale antialiasing, as macOS does since 10.14. Colour glyphs (COLR,
    CBDT, sbix, SVG) go through Vello's glyph renderer.
  - Shadows are Gaussian with σ = blur radius / 2, as CSS `box-shadow`. Vello's blurred rounded
    rectangle falls off as if its `std_dev` were σ/√2 (8 px outside an edge it gives 0.157 of the
    colour where a Gaussian with σ = 12 gives 0.252), so the toolkit passes σ·√2; edges then
    match the Gaussian, corners are within 3/255 at full opacity.
- **Text.** Laid out by Parley: HarfRust shaping with the font's kerning, bidi, ICU line breaking
  with dictionaries for Thai, Lao, Khmer and Burmese, and fallback through fontconfig (loaded at
  run time). The face is Inter, and Inter Display from 20 px, as macOS switches from SF Text to
  SF Display at 20 pt. Baselines sit on whole pixels, as in AppKit.
- **Windows.** `Window` opens an `xdg_toplevel`, a `zwlr_layer_surface_v1` or a session-lock
  surface through libwayland, loaded at run time, so that Vulkan's WSI can present on the same
  connection. The first frame is drawn by `CpuRenderer` into a `wl_shm` buffer while a thread
  creates the Vulkan device and a surface on the window; once it is ready, frames are drawn by
  `GpuRenderer` straight into the surface (mailbox when offered). Frames are paced by
  `wl_surface.frame`. Windows follow the output's fractional scale through `wp_viewporter`.
  Text input v3 and the clipboard are supported.
  - **Window frame.** A toplevel gets its frame from the toolkit, as an `NSWindow` does: a
    28 pt title bar with the title centred in semibold and close, minimise and zoom at the
    top left, rounded corners and a shadow drawn into a 36 pt transparent margin outside the
    window geometry (`set_window_geometry`). The controls are grey in inactive windows and
    turn coloured with their symbols while the pointer is over the group, also in inactive
    windows; they act on release inside. A drag on the bar moves the window, a double click
    zooms. A 6 pt band outside every edge resizes, with corners reaching 14 pt along the
    edges, and the pointer shows the direction. The input region covers the window and the
    band only, so clicks on the shadow reach the window below. Maximised, tiled and full-screen
    windows have no margin, and full screen hides the title bar. `Window::size` and pointer
    positions are in the content's coordinates; the app draws only its content. The title bar
    and its buttons are published to screen readers above the app's tree. `UNBOUND_RENDERER=cpu` keeps a window on
  `CpuRenderer`. `crates/testing/sample` is a settings card on the toolkit that `ci/perf` starts
  to measure.
- **Widgets.** `ui` holds the widget tree: `Stack` (rows and columns with spacing and padding; a
  column stretches its children across, a row centres them), `Label`, `Button`, `Toggle`,
  `Slider`, `TextField` and `SecureField`, laid out, painted into a `Scene` and driven by `Ui`.
  Sizes follow macOS's regular controls: push buttons and text fields 22 px high, switches
  38×22 px (`NSSwitch`), a 3 px focus ring in the accent colour. Buttons and switches act on
  release inside, as in AppKit. Tab moves focus between text fields only unless
  `keyboard_navigation` is set, as macOS's Keyboard navigation setting (off by default) does.
  Text fields edit with Parley's `PlainEditor` and the bindings of the
  [keyboard model](overview.md#keyboard-model) with Super as Command: Super+Left/Right to the
  line ends, Alt+Left/Right by word, Shift extending, Up/Down to the ends of a single-line field
  (as `NSTextField`), Alt+Backspace and Super+Backspace deleting a word and to the line start,
  Super+A selecting all.
- **Accessibility.** Every widget produces an AccessKit node: role, label or value, bounds,
  toggled state or numeric range, and its actions. `Ui::accessibility` returns the `TreeUpdate`
  with the focus, and `Ui::perform` carries out Focus, Click, SetValue, Increment, Decrement and
  ReplaceSelectedText requests. The tree is published on AT-SPI through `accesskit_unix`.
- **Input.** `wl_seat` up to version 9. Pointer events carry the button (primary, secondary,
  middle, back, forward), the click count (a press within 500 ms and 4 pt of the previous press
  of the same button counts up) and the modifiers; Ctrl+primary is a secondary click. Scrolling
  arrives as `Event::Scroll` with its source (wheel, finger, continuous) and phase (a finger
  scroll begins, changes and ends at `axis_stop`, so momentum can start from its end).
  Modifier-only changes arrive as `Event::ModifiersChanged`. `Ui` sends `PointerEnter` and
  `PointerLeave` to every widget on the hit path, bubbles scrolling from the innermost widget
  outwards, and reports the hovered widget's cursor, which `Window::set_cursor` shows through
  `wp_cursor_shape_v1`. A close request (close button, Super+W) asks `App::should_close`, so an
  app can keep its window open, ask first or close a tab instead; `Window::close` closes it.
  Text fields select a word on double click and everything on triple click.

Since `wayland-backend` is built with `client_system`, every Wayland client in a workspace
build, `unbound-test-wayland` included, goes through libwayland.
