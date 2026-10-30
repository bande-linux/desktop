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
- **Symbolic icons.** The theme's symbolic icons are a single fill of light `ink`;
  `Image::tinted(colour)` keeps an image's shape and gives it one colour, so a symbol follows
  `ink2`, the accent when selected, or `on_accent` on a selection.
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
  `GpuRenderer` into a kept frame texture that is copied to the surface (mailbox when offered). Frames are paced by
  `wl_surface.frame`. Windows follow the output's fractional scale through `wp_viewporter`.
  All windows of a process share one Vulkan device, opened with the first and closed with the
  last. Each renderer's image atlas starts at the surface's size (rounded up to 256 px) and grows
  only when an image does not fit. A window hidden for 30 s drops its renderer, frame textures,
  `wl_shm` buffers and, with the last one, the device; showing it again draws the first frames on
  the CPU while the GPU comes back. Every frame is diffed against the last one item by item
  (`damage`): only the changed regions are redrawn (clipped on the GPU into a kept frame texture,
  rendered region by region on the CPU) and only they are damaged on `wl_shm` buffers.
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
    `chassis` for the app to draw on, and the toolkit paints the sheet in the rest. An app whose
    toolbar lives in the header calls `Window::set_header(Header { title_x, items })`: its
    content then also covers the header (`Window::header_height` is 0 in full screen), the
    toolkit draws the title at `title_x` (clipped before the next item), presses on `items`
    (content coordinates) go to the app, and the rest of the header still moves, zooms and
    opens the window menu. `framed_scene` composes an app's scene with the frame for goldens. The header
    and its buttons are published to screen readers above the app's tree. Server-side frames
    for third-party apps (`compositor/src/frame.rs`) draw the same header and pill from
    `title_bar.rs`, 40 pt high, and redraw when Alt is pressed or released so the zoom glyph
    follows it. Goldens: `platform/tests/golden/window-frame-*.png`, including an inactive
    window behind a second one. `UNBOUND_RENDERER=cpu` keeps a window on
  `CpuRenderer`. `crates/testing/sample` is a settings card on the toolkit that `ci/perf` starts
  to measure.
- **Widgets.** `ui` holds the widget tree: `Stack` (rows and columns with spacing and padding; a
  column stretches its children across, a row centres them), `Label`, `Shortcut`, the controls
  below, `Table`, `ScrollView`, `Disclosure` and `FreeLayout`, laid out, painted into a `Scene`
  and driven by `Ui`. Buttons and switches act on release inside, as in AppKit. Tab moves focus
  between text fields only unless `keyboard_navigation` is set, as macOS's Keyboard navigation
  setting (off by default) does; tabbing into a field selects its text. A widget opens a menu
  of its own (a pop-up button's choices) with `Context::show_menu`; the choice comes back to it
  through `menu_action`.
- **Controls** (Datum §9, golden `widgets/tests/golden/controls-*.png`). Regular controls are
  28 pt high (`ControlSize::Regular`), radius 6 (`Radius::S`). Focus is a 2 pt `focus` ring at a
  1 pt gap (radius + 3); fields instead turn their outline into a 1.5 pt accent line and add the
  ring at 35 %. Text and glyphs on the accent use `on_accent`, on danger `on_danger` (dark ink
  on the light accents of dark mode).

  | Control | Look and API |
  |---|---|
  | `Button::new(title, action)` | secondary: `well` fill, `ink` 13/500, 8 % ink on hover, 16 % pressed |
  | `.primary()` | accent fill, `on_accent` 13/600, Return; press mixes 15 % black |
  | `.cancel()` | secondary look, Escape |
  | `.quiet()` | `accent_text` 600, `well` fill on hover only |
  | `.destructive()` | danger fill, `on_danger` text; only for the destructive verb |
  | `Button::symbol(image, label, action)` | 28×28 icon button: a symbolic `Image` tinted `ink2` (`ink` on hover), `well` on hover; `label` is its tooltip and accessible name |
  | `TextField`, `TextView`, `SecureField` | `field` fill (`overlay` light, `well` dark), 1 pt `edge` outline, placeholder `ink3`, 1.5 pt accent caret, selection `accent_soft` (`well` when unfocused); `SecureField` shows a Caps Lock sign while focused with Caps Lock on (`Context::caps_lock`, set by `Window::drive`) |
  | `SearchField` | `well` fill, radius 8, no outline, magnifier `ink3`, clear button |
  | `ComboBox` | a text field with a single down chevron |
  | `Toggle` (switch) | 36×20 pill, `edge` off, accent on, white 15 pt knob with a small shadow |
  | `Checkbox` | 16 pt, radius 4, 1.5 pt `ink3` outline; on: accent with a square-capped check |
  | `RadioGroup` | 16 pt circles; on: accent with a 5 pt square centre |
  | `SegmentedControl` | `well` track radius 8, a raised chip radius 6 (`raised`: `overlay` light, `edge` dark) that glides on `quick`; selected label 600 `ink`; `set_selected(cx, index)` from code |
  | `PopUpButton` | `well` fill, single down chevron, the menu opens with the current choice over the button |
  | `Stepper` | one 96 pt `well` capsule: − value + (tabular figures); `.format(fn)` formats the value |
  | `Slider` | 4 pt `well` track, accent fill, a 7×20 white fader knob with an `edge` outline; `.ticks(n)` adds `ink3` tick marks and detents the knob settles on (`interactive` spring); `set_value(cx, value)` from code |
  | `ProgressBar`, `LevelIndicator` | 24 cells with 2 pt gaps, radius 1, accent on `well`; indeterminate: a 4-cell block gliding back and forth (pulsing with Reduce Motion) |
  | `Spinner` | 12 `ink2` spokes |
  | `Disclosure` | chevron that turns, content fades in |
  | Tooltip | `ink` fill with `chassis` text 12/500, radius 6, no arrow |
  | `Shortcut::new("Alt+Super+K")` | key symbols in `ink2`, with the Super glyph (a 9.5 pt rounded square with a 3 pt square inside) instead of ⌘; `shortcut_keys` splits a shortcut into modifier symbols, Super and key |

- **Menus** (golden `platform/tests/golden/menu-submenu-*.png`). Opaque `overlay`, radius 10
  (`MENU_RADIUS`) with the Datum overlay shadow pair and no blur; 4 pt inset, 28 pt rows, 9 pt
  separators in `hairline`, the highlight accent radius 6 with `on_accent` text, disabled items
  `ink3`, the shortcut column right-aligned in `ink3` with the Super glyph, square-capped
  checkmarks, submenu chevrons. Searchable menus (`Menu::searchable`) have a `well` search
  field; Alt swaps items for their `alternate`.
- **Alerts and sheets** (golden `platform/tests/golden/alert-*.png`). `Alert` is a card of radius
  14: the body on `overlay` with an optional 36 pt icon (`.icon(image)`), a Manrope `Heading`
  and the message in `ink2`; a footer tray on `chassis` holds the buttons: default at the right
  (`.primary` or `.destructive`), cancel left of it, alternatives (`.other`) as quiet buttons at
  the far left. When they do not fit side by side they stack, default on top. The parent dims
  with `scrim` (18 % light, 40 % dark); `Ui::present_sheet` drops the card from under the header,
  `Ui::present_dialog` centres it.
- **Tables** (golden `platform/tests/golden/table-*.png`). Rows 32, header 28 with `Label`-style
  tracked capitals in `ink3` (`ink2` for the sorted column) over a `hairline` underline, no
  stripes and no column separators. Contiguous selected rows form one block of radius 6 inset
  4 × 1: accent with `on_accent` text while focused, `well` otherwise. A single selected run
  glides to its new rows on `quick`. Scroll bars are a 6 pt `ink3` knob that fades.
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
