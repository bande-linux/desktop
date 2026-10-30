# Toolkit

Every Unbound shell client and app draws with one shared toolkit, `unbound-toolkit`, so that
look, shortcuts and accessibility are the same everywhere. It draws with Vello and lays out text
with Parley ([`../decisions/0001-rendering.md`](../decisions/0001-rendering.md), Decision 2).
How it grows to AppKit depth is decided in
[`../decisions/0002-toolkit.md`](../decisions/0002-toolkit.md).

## Crates

- `unbound-design` (`crates/toolkit/design`): the [Datum](../design/datum.md) design tokens:
  colours, radii, spacing, control sizes, type roles and elevation. No dependencies, so a
  service (the portal, a GTK/Qt theme exporter) can use the palette without the renderer.
- `unbound-graphics` (`crates/toolkit/graphics`): scene, Vello CPU and GPU renderers, Parley
  text, the shared window header (`title_bar.rs`) and the input event types; it re-exports the
  tokens. No Wayland.
- `unbound-widgets` (`crates/toolkit/widgets`): the widget tree and widgets, on graphics only,
  so widget tests need no compositor.
- `unbound-toolkit-platform` (`crates/toolkit/platform`): Wayland windows and roles, seat input,
  clipboard, text input, capture, the run loop, accessibility publication and appearance.
- `unbound-toolkit` (`crates/toolkit/toolkit`): the facade apps depend on; `ui` is
  `unbound-widgets`.

CI fails when a service depends on the widgets, platform or facade, or when graphics or widgets
depend on Wayland.

## Appearance

Every window follows the system appearance itself: the platform reads `color-scheme`,
`accent-color` and `contrast` of `org.freedesktop.appearance` from the XDG Settings portal, at
window creation and live on `SettingChanged`, and delivers `WindowEvent::AppearanceChanged`.
The portal's accent is mapped to the nearest of Datum's eight accents (by hue and chroma in
OKLab), so the palette never holds an untested colour. Apps never read `config.kdl` for it.

## Design tokens

Every colour, radius, size and type style comes from `unbound-design`; no app or shell surface
writes a hex value, a corner radius or a font name of its own.

- **Colours.** `Appearance::palette()` (or `Palette::generate(scheme, accent, contrast)`)
  returns the roles, generated from OKLCH with Datum's warm neutral hue (80°):

  | Role | Use |
  |---|---|
  | `chassis` | window frame, header, sidebar, bar |
  | `sheet` | the content inside a window (a `Scene`'s background) |
  | `overlay` | menus, popovers, notifications, field fills in light mode |
  | `well` | control fills, the window-control pill, unfocused selection |
  | `hairline` | separators, the dark sheet edge |
  | `edge` | window and control outlines (≥ 1.9:1, ≥ 3:1 with Increase Contrast) |
  | `ink`, `ink2`, `ink3` | primary, secondary and tertiary text and glyphs |
  | `accent`, `accent_text`, `on_accent`, `accent_soft`, `focus` | the one signal colour: selection, focus, links; `on_accent` is white or dark ink, whichever reads better (dark in dark mode) |
  | `danger`, `warning`, `success` | status only |
  | `scrim` | the dimming behind modal moments |

  `Accent` has eight presets (Ultramarine by default, Teal, Moss, Amber, Ember, Rose, Plum,
  Graphite); `Contrast::Increased` darkens `edge`. The tests in `crates/toolkit/design/tests`
  check every scheme × accent × contrast: text roles against WCAG and APCA, control boundaries,
  the accent and danger on every surface and text on the accent. A token change that breaks
  legibility fails CI.
- **Radii.** `Radius::{XS, S, M, L, XL}` = 4, 6, 8, 10, 14 pt; a child inside a rounded parent
  takes `Radius::concentric(outer, inset)` (= outer − inset, at least 4): the 14 pt window with
  its sheet inset 6 gives the sheet 8.
- **Spacing.** `Spacing` 2, 4, 8, 12, 16, 24, 32, 48; `ControlSize::{Small, Regular, Large}`
  heights 22, 28, 36.
- **Type.** `TextStyle::role(TextRole::…, colour)` sets face, size, line height, weight,
  tracking and capitals: `Display` 28/34, `Title` 22/28, `Heading` 17/22 and `WindowTitle`
  15/20 in Manrope Bold; `Body` 13/18, `BodyStrong` 13/18 semibold, `Callout` 12/16 and
  `Label` 10.5/14 semibold tracked capitals in Inter; `Mono` 13/18 in JetBrains Mono.
  `TextStyle::new(size, colour)` remains for free sizes: Inter below 20 pt, Manrope from 20 pt.
  `.tabular()` adds `tnum` for numbers in columns.
- **Elevation.** `Elevation::{Frame, Overlay}.shadows(scheme, active)` gives the key and ambient
  shadow pair (offset, σ, opacity); an inactive frame's ambient shadow is half as strong.

## Goals

- **Semantic colour tokens**: the Datum roles above for light and dark, switched live when the
  appearance changes.
- **Accessibility built in**: every widget produces and maintains an AccessKit node, published
  on AT-SPI, so screen readers work by construction.
- **Standard controls**: scroll views with spring physics, text fields, sliders, switches and
  tab views with the same keyboard and pointer behaviour in every app.

## Implemented (`crates/toolkit/toolkit`)

- **Tokens.** See [Design tokens](#design-tokens).
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
  run time). Interface text is Inter with Datum's alternates on (`cv05` l with a tail, `cv08` I
  with serifs, `ss07`/`ss08` square punctuation and quotes); display text (window titles,
  headings, 20 pt and up) is Manrope; monospace is JetBrains Mono with a slashed zero. A missing
  face falls back (Manrope to Inter, JetBrains Mono to DejaVu Sans Mono, then fontconfig's
  generic family), so nothing breaks without the fonts; `unbound-desktop` depends on
  `fonts-inter`, `fonts-manrope` and `fonts-jetbrains-mono`. Baselines sit on whole pixels, as
  in AppKit.
- **Windows.** `Window` opens an `xdg_toplevel`, a `zwlr_layer_surface_v1` or a session-lock
  surface through libwayland, loaded at run time, so that Vulkan's WSI can present on the same
  connection. The first frame is drawn by `CpuRenderer` into a `wl_shm` buffer while a thread
  creates the Vulkan device and a surface on the window; once it is ready, frames are drawn by
  `GpuRenderer` straight into the surface (mailbox when offered). Frames are paced by
  `wl_surface.frame`. Windows follow the output's fractional scale through `wp_viewporter`.
  Text input v3 and the clipboard are supported.
  - **Window frame.** A toplevel gets its frame from the toolkit, in Datum's frame and sheet:
    the window is `chassis`, with a 48 pt header holding the window controls at the left and
    the title flush left after them in Manrope 15/20 bold (no separator line), and the content
    is a `sheet` inset 6 pt left, right and bottom, rounded 8 (concentric with the window's 14),
    with a `hairline` edge in dark mode. The controls are one 66×24 pill (`well`) with ×, − and
    corner brackets always visible in `ink2`; over the pill they turn `ink`, and the close cell
    alone gets a red disc with a white ×. A press shows an `ink` disc at 12 %. In inactive
    windows the pill is a `hairline` outline with `ink3` glyphs. The zoom glyph turns inward
    while the window is zoomed; with Alt held it shows + and Alt-click enters full screen. The
    hit cells cover the pill's full height and the close cell reaches the window's left edge.
    Rounded corners (14 pt), the 55 % `edge` ring and the shadow pair are the compositor's, cut
    at the window geometry (`set_window_geometry`), so every app looks alike; maximised, tiled
    and full-screen windows are square. They act on release inside. A drag on the header or
    the chassis around the sheet moves the window, a double click zooms, a right click opens
    the window menu. A transparent 6 pt band outside every edge resizes, with corners reaching
    14 pt along the edges, and the pointer shows the direction. Maximised, tiled and
    full-screen windows have no band, and full screen hides the header and the inset.
    `Window::size` and pointer positions are in the content's coordinates: by default the
    content is the sheet and the app draws only into it (clipped to its rounded shape, on the
    `Scene`'s background). An app with a sidebar or toolbar on the frame calls
    `Window::set_frame_regions(FrameRegions { sidebar, toolbar })`: its content then covers
    everything below the header, the left `sidebar` points and the top `toolbar` points stay on
    `chassis` for the app to draw on, and the toolkit paints the sheet in the rest. The header
    and its buttons are published to screen readers above the app's tree. Server-side frames
    for third-party apps (`compositor/src/frame.rs`) draw the same header and pill from
    `title_bar.rs`, 40 pt high. `UNBOUND_RENDERER=cpu` keeps a window on
  `CpuRenderer`. `crates/testing/sample` is a settings card on the toolkit that `ci/perf` starts
  to measure.
- **Widgets.** `ui` holds the widget tree: `Stack` (rows and columns with spacing and padding; a
  column stretches its children across, a row centres them), `Label`, `Button`, `Toggle`,
  `Checkbox` (on, off, mixed), `RadioGroup`, `SegmentedControl` (its selection slides on a
  spring), `PopUpButton` (a menu with the current choice over the button), `ComboBox`,
  `Stepper` (repeats while held), `Slider`, `ProgressBar` (determinate or indeterminate),
  `Spinner`, `TextField`, `SearchField` (magnifier, clear button, Escape clears, reports every
  change), `TextView` and `SecureField`, laid out, painted into a `Scene` and driven by `Ui`.
  Sizes follow macOS's regular controls: push buttons and text fields 22 px high, switches
  38×22 px (`NSSwitch`), a 3 px focus ring in the accent colour. Buttons and switches act on
  release inside, as in AppKit. Tab moves focus between text fields only unless
  `keyboard_navigation` is set, as macOS's Keyboard navigation setting (off by default) does;
  tabbing into a field selects its text. A widget opens a menu of its own (a pop-up button's
  choices) with `Context::show_menu`; the choice comes back to it through `menu_action`.
- **Tooltips.** A widget that returns `Widget::tooltip` (buttons and pop-up buttons through
  `.tooltip(...)`) shows it after the pointer rests on it for `TOOLTIP_DELAY`, or after Tab
  moves focus to it. It fades in under the pointer inside the window, a click or key hides it
  until the pointer leaves, and moving to a neighbour while one is shown shows the next at
  once. `Ui::deadline` tells the window when to draw it; `Window::drive` wakes for it. The text
  is also the node's accessible description.
- **Text.** `TextField`, `SearchField`, `ComboBox` and `TextView` share one editor
  (`editor.rs`). The document is a list of paragraphs, each laid out by Parley on its own and
  only when visible or under the caret, with estimated heights for the rest, so a 100,000-line
  `TextView` types without delay. Movement uses Parley's cursors, so arrows move visually
  through mixed left-to-right and right-to-left text and each paragraph takes its own
  direction. Bindings follow the [keyboard model](overview.md#keyboard-model) with Super as
  Command: Super+Left/Right and Home/End to the line ends, Super+Up/Down to the document ends,
  Alt+Left/Right by word, Alt+Up/Down by paragraph, Up/Down by line keeping the column (to the
  ends of a single-line field, as `NSTextField`), Page Up/Down by a page, Shift extending,
  Alt+Backspace/Delete and Super+Backspace deleting a word and to the line start, Super+A,
  Super+X/C/V. Super+Z and Shift+Super+Z undo and redo; typing undoes a word at a time, and
  deleting, pasting, cutting and composing each undo as one step. Double click selects a word,
  triple click a paragraph, and dragging after either extends by that unit; dragging past the
  edge scrolls. The context menu offers Undo, Redo, Cut, Copy, Paste and Select All. Input
  method text being composed is shown underlined in place (the clause being converted thicker)
  without entering the value, the candidate window is placed through `set_cursor_rectangle`
  under it, and a click while composing keeps the text and resets the input method. Text is
  published to screen readers as text runs with the caret and selection, which they can also
  set. `TextView` wraps, scrolls with a `ScrollState`, keeps the caret in view and can be
  read-only.
- **Animation.** `ui::Animated` holds a value that springs to its target with a preset from
  `unbound-spring` (`Spring::QUICK`, `STANDARD`, `GENTLE`, `FADE`, ...). A new target retargets
  from the current position and velocity, so a reversed action never jumps or queues. Values
  are sampled at the frame's time; while any value moves, `Ui::is_animating` is true and an app
  returns it from `App::animating`, which asks for the next frame after each one drawn. With
  Reduce Motion (`reduced-motion` of `org.freedesktop.appearance`, in
  `Appearance::reduce_motion`, applied by `Ui::set_appearance`) every spring but `FADE` jumps
  to its target, so movement is gone and fades stay. Switches slide their knob and blend their
  track; the window controls' symbols fade in.
- **Accessibility.** Every widget produces an AccessKit node: role, label or value, bounds,
  toggled state or numeric range, and its actions. `Ui::accessibility` returns the `TreeUpdate`
  with the focus, and `Ui::perform` carries out Focus, Click, SetValue, Increment, Decrement,
  ReplaceSelectedText and SetTextSelection requests. The tree is published on AT-SPI through `accesskit_unix`.
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

Since `wayland-backend` is built with `client_system`, every Wayland client in a workspace
build, `unbound-test-wayland` included, goes through libwayland.
