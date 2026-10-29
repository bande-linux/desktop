# Toolkit

Every Unbound shell client and app draws with one shared toolkit, `unbound-toolkit`, so that
look, shortcuts and accessibility are the same everywhere. It draws with Vello and lays out text
with Parley ([`../decisions/0001-rendering.md`](../decisions/0001-rendering.md), Decision 2).
How it grows to AppKit depth is proposed in
[`../decisions/0002-toolkit.md`](../decisions/0002-toolkit.md).

## Goals

- **Semantic colour tokens**: colour roles (`surfaceBackground`, `surfaceElevated`,
  `labelPrimary`, `labelMuted`, `accentColor`, `separatorColor`) for light and dark, switched
  live when the appearance changes.
- **Accessibility built in**: every widget produces and maintains an AccessKit node, published
  on AT-SPI, so screen readers work by construction.
- **Standard controls**: scroll views with spring physics, text fields, sliders, switches and
  tab views with the same keyboard and pointer behaviour in every app.

## Implemented (`crates/unbound-toolkit`)

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
  Text input v3 and the clipboard are supported. `UNBOUND_RENDERER=cpu` keeps a window on
  `CpuRenderer`. `crates/unbound-sample` is a settings card on the toolkit that `ci/perf` starts
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
- **Input.** Pointer input is limited today: only the left button, no click count or modifiers
  on pointer events, no hover per widget and no popups. The order in which this grows is in
  [decision 0002](../decisions/0002-toolkit.md).

Since `wayland-backend` is built with `client_system`, every Wayland client in a workspace
build, `unbound-test-wayland` included, goes through libwayland.
