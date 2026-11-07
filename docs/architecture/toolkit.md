# Toolkit

Every Bande shell client and app draws with one shared toolkit, `bande-toolkit`, so that
look, shortcuts and accessibility are the same everywhere. It draws with Vello and lays out text
with Parley ([`../decisions/0001-rendering.md`](../decisions/0001-rendering.md), Decision 2).
How it grows to AppKit depth is decided in
[`../decisions/0002-toolkit.md`](../decisions/0002-toolkit.md).

## Crates

- `bande-design` (`crates/toolkit/design`): the [Datum](../design/datum.md) design tokens:
  colours, radii, spacing, control sizes, type roles and elevation. No dependencies, so a
  service (the portal, a GTK/Qt theme exporter) can use the palette without the renderer.
- `bande-graphics` (`crates/toolkit/graphics`): scene, Vello CPU and GPU renderers, Parley
  text, the shared window header (`title_bar.rs`) and the input event types; it re-exports the
  tokens. No Wayland.
- `bande-widgets` (`crates/toolkit/widgets`): the widget tree and widgets, on graphics only,
  so widget tests need no compositor.
- `bande-toolkit-platform` (`crates/toolkit/platform`): Wayland windows and roles, seat input,
  clipboard, text input, capture, the run loop, accessibility publication and appearance.
- `bande-toolkit` (`crates/toolkit/toolkit`): the facade apps depend on; `ui` is
  `bande-widgets`.

`ci/release/lint.sh` fails when a service other than the polkit agent depends on the widgets,
platform or facade, or when graphics or widgets depend on Wayland.

## Appearance

Every window follows the system appearance itself: the platform reads `color-scheme`,
`accent-color` and `contrast` of `org.freedesktop.appearance` from the XDG Settings portal, at
window creation and live on `SettingChanged`, and delivers `WindowEvent::AppearanceChanged`.
The portal's accent is mapped to the nearest of Datum's eight accents (by hue and chroma in
OKLab), so the palette never holds an untested colour. Apps never read `config.kdl` for it.

## Design tokens

Every colour, radius, size and type style comes from `bande-design`; no app or shell surface
writes a hex value, a corner radius or a font name of its own.

- **Colours.** `Appearance::palette()` (or `Palette::generate(scheme, accent, contrast)`)
  returns the roles, generated from OKLCH with Datum's neutral hue (warm 80° in light, dusk
  slate 262° in dark):

  | Role | Use |
  |---|---|
  | `chassis` | window frame, header, sidebar, bar |
  | `sheet` | the content inside a window (a `Scene`'s background) |
  | `overlay` | menus, popovers, notifications, field fills in light mode |
  | `well` | control fills, the window-control pill, unfocused selection |
  | `hairline` | separators, the dark sheet edge |
  | `edge` | control outlines (≥ 1.9:1, ≥ 3:1 with Increase Contrast) |
  | `ink`, `ink2`, `ink3` | primary, secondary and tertiary text and glyphs |
  | `accent`, `accent_text`, `on_accent`, `accent_soft`, `focus` | the one signal colour: selection, focus, links; `on_accent` is white or dark ink, whichever reads better (dark in dark mode) |
  | `danger`, `warning`, `success` | status only |
  | `scrim` | the dimming behind modal moments |

  `Accent` has eight presets (Ultramarine by default, Teal, Moss, Amber, Ember, Rose, Plum,
  Graphite); `Contrast::Increased` darkens `edge`. Every scheme × accent × contrast was solved
  for WCAG and APCA contrast ([Datum §5](../design/datum.md#5-colour)); a token change keeps
  those targets.
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
  `TextSystem::fitted(text, style, width)` lays text out on one line, cut at the end with
  "…" when wider (`shortened` with `Cut::Middle` keeps the tail).
- **Symbolic icons.** The theme's symbolic icons are a single fill of light `ink`;
  `Image::tinted(colour)` keeps an image's shape and gives it one colour, so a symbol follows
  `ink2`, the accent when selected, or `on_accent` on a selection.
- **Elevation.** `Elevation::{Frame, Overlay}.shadows(scheme, active)` gives the key and ambient
  shadow pair (offset, σ, opacity); an inactive frame's ambient shadow is half as strong.
  `Item::shadows(shape, elevation, scheme, active)` turns the pair into the two shadow items
  under a shape, and `ui::focus_ring_for(palette, shape, items)` draws the focus ring around a
  custom control.

## Implemented (`crates/toolkit/toolkit`)

- **Tokens.** See [Design tokens](#design-tokens).
- **Scene.** A `Scene` of fills, strokes, linear, radial and conic gradients, rounded
  rectangles, clips, opacity and blend layers (`Item::Blend`, the CSS `mix-blend-mode` set),
  2D transforms (`Item::Transform`), shadows, text, images (`Image::from_rgba` or
  `from_premultiplied`, uploaded once per renderer and freed when the last `Image` drops;
  `Image::shared` wraps an app's premultiplied `Arc<[u8]>` and copies it only while a renderer
  needs a CPU pixmap, so the GPU path keeps no second copy; `Item::Tiles` repeats one), and glyph runs laid out elsewhere (`Item::Glyphs`, e.g. a web page's, drawn
  from their own `FontData` without reshaping), which `CpuRenderer` (`vello_cpu`) and
  `GpuRenderer` (`vello_gpu` on Vulkan) draw from one encoder.
  - The encoder records items into drawing ops (flattened paths, glyph outlines, paints) and
    replays them. A `Picture` is a retained sub-scene: each renderer records it once, keeps the
    ops until the last clone drops, and skips it when its bounds are off screen. A long document
    split into pictures per tile scrolls by translating them, without rebuilding or re-recording.
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
  headings, 20 pt and up) is Manrope; monospace is JetBrains Mono with a slashed zero, or the
  family a `TextSystem` was given with `set_monospace_family` (Terminal's font). A missing
  face falls back (Manrope to Inter, JetBrains Mono to DejaVu Sans Mono, then fontconfig's
  generic family), so nothing breaks without the fonts; `bande-desktop` depends on
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
    corner brackets in `ink2`, and the menu button its own 24×24 `well` beside it. Each control
    is its own button with the same 18 pt disc: pointing at one fades in a backfill of `ink` at
    12 % and turns its glyph `ink` (the close control a `danger` disc with an `on_danger` ×), and
    a press deepens it to 22 % (`danger` darkened by 15 %). A disabled zoom (fixed-size windows)
    has a faded glyph and no hover or press. In inactive windows the wells are a `hairline`
    outline with `ink3` glyphs; hover and press work the same, since a click on an inactive
    window's control acts at once. The zoom glyph turns inward
    while the window is zoomed; with Alt held it shows + and Alt-click enters full screen. The
    hit cells cover the pill's full height and the close cell reaches the window's left edge.
    Rounded corners (16 pt), the one pixel dark ring and the shadow pair are the compositor's, cut
    at the window geometry (`set_window_geometry`), so every app looks alike; only full-screen
    windows are square. They act on release inside. A drag on the header or
    the chassis around the sheet moves the window, a double click zooms, a right click opens
    the window menu. A band from the rounded outline to 8 pt outside resizes, corner
    cut-outs included, with corners reaching 32 pt along the edges and 16 pt out diagonally, the part
    beyond the 8 pt margin being the compositor's, and the pointer shows the direction; nothing inside the outline resizes, so the free space around tabs moves the
    window. Tiled windows have the same band, a side with no room outside gets its band from
    the compositor, maximised and full-screen windows have none, and full screen hides the header and the inset.
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
    opens the window menu. The first item starts at `HEADER_LEADING`, the same place a plain
    title starts. A strip or toolbar in a row below the header names its free space with
    `Window::set_move_areas` (content coordinates), which then behaves as the header does. The header and its buttons are published to screen readers above the app's tree. Server-side frames
    for third-party apps (`compositor/src/frame.rs`) draw the same header and pill from
    `title_bar.rs`, 40 pt high, and redraw when Alt is pressed or released so the zoom glyph
    follows it.
  - **Resizing from the app.** As `NSWindow setFrame:display:animate:`,
    `Window::resize_content(size, Sizing)` gives a toplevel a new content size and makes it
    `Sizing::Fixed` (minimum and maximum size equal, no resize band or zoom) or
    `Sizing::Resizable`. A floating window takes the size in its next frame; a zoomed, tiled or
    full-screen one keeps the size it was given (a fixed one is then released to floating by the
    compositor). A fixed floating window ignores configure sizes, so its own size wins. When the only Vulkan device renders on the CPU (lavapipe or
  llvmpipe, as in VMs) every window stays on `CpuRenderer` and presents `wl_shm` buffers with
  only the changed regions damaged; `BANDE_RENDERER=gpu` or `=cpu` overrides the choice. A
  window whose background is opaque declares its window rectangle as its opaque region, so
  the compositor shades nothing beneath it. `crates/testing/sample` is a settings card on the toolkit that `bande-bench` starts
  to measure.
- **Widgets.** `ui` holds the widget tree: `Stack` (rows and columns with spacing and padding; a
  column stretches its children across, a row centres them), `Label`, `Shortcut`, the controls
  below, `Table`, `ScrollView`, `Disclosure` and `FreeLayout`, laid out, painted into a `Scene`
  and driven by `Ui`. `Ui::update_widget(id, |widget, cx| ...)` changes a widget from code with the context it needs (e.g. `SegmentedControl::set_selected`, `Slider::set_value`), so the change animates. Buttons and switches act on release inside, as in AppKit. Tab moves focus
  between text fields only unless `keyboard_navigation` is set, as macOS's Keyboard navigation
  setting (off by default) does; tabbing into a field selects its text. A widget opens a menu
  of its own (a pop-up button's choices) with `Context::show_menu`; the choice comes back to it
  through `menu_action`.
- **Controls** (Datum §9). Regular controls are
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
  | `Button::symbol(image, label, action)` | 28×28 icon button: a symbolic `Image` tinted `ink2` (`ink` on hover), `well` on hover; `label` is its tooltip and accessible name; `set_enabled(false)` dims any button to `ink3` and ignores it |
  | `TextField`, `TextView`, `SecureField` | `field` fill (`overlay` light, `well` dark), 1 pt `edge` outline, placeholder `ink3`, 1.5 pt accent caret, selection `accent_soft` (`well` when unfocused); `SecureField` shows a Caps Lock sign while focused with Caps Lock on (`Context::caps_lock`, set by `Window::drive`), and Ctrl+Backspace clears it |
  | `SearchField` | `well` fill, radius 8, no outline, magnifier `ink3`, clear button |
  | `TabStrip` | the tab bar for apps with tabs: in the header (Terminal, Browser), or `TabStripLayout::below` the toolbar on the sheet (Files): 28 pt tabs sharing the width left of a + button, the active one on `well`, close cross and badge in the leading 16 pt; press selects, dragging reorders with springs, dragging out of the strip returns `TabAction::Detach`; `TabStripLayout` gives the slots, `slide_to` and `insertion_index` serve a tab dragged in from another window |
  | `ComboBox` | a text field with a single down chevron |
  | `TypeSelect` | what has been typed for type-to-select, lowercased, starting over after a one-second pause; the caller does the matching (`Table`, menus, Files, the desktop) |
  | `Toggle` (switch) | 36×20 pill, `edge` off, accent on, white 15 pt knob with a small shadow |
  | `Checkbox` | 16 pt, radius 4, 1.5 pt `ink3` outline; on: accent with a square-capped check; disabled: 45 % opacity, no input, insensitive to accessibility |
  | `RadioGroup` | 16 pt circles; on: accent with a 5 pt square centre |
  | `SegmentedControl` | `well` track radius 8, a raised chip radius 6 (`raised`: `overlay` light, `edge` dark) that glides on `quick`; selected label 600 `ink`; `set_selected(cx, index)` from code; `SegmentedControl::symbols` shows 16 pt symbols (`ink2`, `ink` when selected) in 32 pt segments, each with its accessible name; `fill_equally()` splits the placed frame into equal segments (a toolbar slot) |
  | `PopUpButton` | `well` fill, single down chevron, the menu opens with the current choice over the button |
  | `Stepper` | one 96 pt `well` capsule: − value + (tabular figures); `.format(fn)` formats the value |
  | `Slider` | 4 pt `well` track, accent fill, a 7×20 white fader knob with an `edge` outline; `.ticks(n)` adds `ink3` tick marks and detents the knob settles on (`interactive` spring); `set_value(cx, value)` from code |
  | `ProgressBar`, `LevelIndicator` | 24 cells with 2 pt gaps, radius 1, accent on `well`; indeterminate: a 4-cell block gliding back and forth (pulsing with Reduce Motion) |
  | `Spinner` | 12 `ink2` spokes |
  | `Disclosure` | chevron that turns, content fades in |
  | Tooltip | `ink` fill with `chassis` text 12/500, radius 6, no arrow |
  | `Shortcut::new("Shift+Ctrl+N")` | the shortcut as words in `ink2`, modifiers in GNOME order (Shift, Ctrl, Alt, Super), arrows as arrow symbols, "Page Up", "Esc"; `shortcut_label` writes the label, `parse_shortcut` reads a shortcut string back into a key and modifiers (`Ctrl` or `Control`), and `shortcut_pressed` tells whether a key event matches one |

- **Menus**. Opaque `overlay`, radius 10
  (`MENU_RADIUS`) with the Datum overlay shadow pair and no blur; 4 pt inset, 28 pt rows, 9 pt
  separators in `hairline`, the highlight accent radius 6 with `on_accent` text, disabled items
  `ink3`, the shortcut column right-aligned in `ink3` as words, square-capped
  checkmarks, submenu chevrons. Searchable menus (`Menu::searchable`) have a `well` search
  field; Alt swaps items for their `alternate`.
- **Alerts and sheets**. `Alert` is a card of radius
  14: the body on `overlay` with an optional 36 pt icon (`.icon(image)`), a Manrope `Heading`
  and the message in `ink2`; a footer tray on `chassis` holds the buttons: default at the right
  (`.primary` or `.destructive`), cancel left of it, alternatives (`.other`) as quiet buttons at
  the far left. When they do not fit side by side they stack, default on top. The parent dims
  with `scrim` (18 % light, 40 % dark); `Ui::present_sheet` drops the card from under the header (`Ui::set_sheet_top` for an app whose content covers the header),
  `Ui::present_dialog` centres it.
- **Tables**. Rows 32 (24 with `Table::compact` for dense lists such as Activity Monitor), header 28 with `Label`-style
  tracked capitals in `ink3` (`ink2` for the sorted column) over a `hairline` underline, no
  stripes and no column separators. Contiguous selected rows form one block of radius 6 inset
  4 × 1: accent with `on_accent` text while focused, `well` otherwise. A single selected run
  glides to its new rows on `quick`. `Table::set_headings` adds group headings (semibold
  `ink` over a `hairline`, one row tall) above the rows that start each group, as Files' Use
  Groups does, and level 2 heading nodes before those rows so screen readers announce each
  group; a selection run never spans a heading. `Row::dimmed` tones a row down one ink step
  with its icons at `Palette::DIMMED_ALPHA`, for normally hidden files. `Column::movable`
  columns can be dragged along the header: the whole column lifts and follows the pointer,
  neighbours slide aside on `standard`, Escape puts it back, and a click without a drag still
  sorts (on release). The first column never moves. Column and `Sort` indices keep the app's
  order, so moving needs no new rows: the app stores what `Table::on_column_order` reports and
  restores it with `set_column_order`; `Table::move_column` is the menu and keyboard way. The
  slide itself is `Reorder`, which `TabStrip` also uses and a hand-drawn list can too. Scroll bars are macOS overlay scrollers
  (`Scrollbar`, also usable on its own by views that scroll themselves, such as Terminal): a
  6 pt `ink3` knob shown while scrolling or when the pointer is in the right 14 pt, widening to
  10 pt over a faint lane on hover; drag the knob, click the lane to page; it fades 0.9 s
  after the last scroll on one deadline, with no wake-ups once gone.
- **Graphs**. `Graph::new(label, kind, span)` plots the last `span` samples of each `Series`
  (values 0–1, `None` for a gap; the newest at the right) in a `well` mixed toward the sheet,
  radius 6, with three `hairline` grid lines. `GraphKind::Line` strokes each series 1.5 pt,
  `Area` also fills under it at 30 %, `Stacked` adds the series up and draws the totals as
  areas, the top first. Colours are `SeriesColor` roles taken from the palette: `Primary`
  (blue) and `Secondary` (red) in OKLCH, and `success`, `warning`, `danger_text`;
  `SeriesColor::color` gives the same colour to the readout beside a graph.
  `.caption(title, span)` adds a 20 pt `Label` caption above the well (title centred, span at
  the right) and a `hairline` outline around it; without one the graph is a bare sparkline.
  `set_note` puts a tabular label in its top-left corner (a scale or a name), `set_bands` 3 pt
  status strips along the bottom (Activity Monitor's charging spans). It never animates or
  wakes: it draws again only when the app sets new data. Activity Monitor draws its graphs
  with it.
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
  direction. Bindings follow the [keyboard model](overview.md#keyboard-model), as in GNOME:
  Home/End to the line ends, Ctrl+Home/End to the document ends, Ctrl+Left/Right by word,
  Ctrl+Up/Down by paragraph, Up/Down by line keeping the column (to the ends of a single-line
  field), Page Up/Down by a page, Shift extending, Ctrl+Backspace/Delete deleting a word,
  Ctrl+A, Ctrl+X/C/V. Ctrl+Z undoes, Shift+Ctrl+Z and Ctrl+Y redo; Alt and Super are not used.
  Typing undoes a word at a time, and
  deleting, pasting, cutting and composing each undo as one step. Double click selects a word,
  triple click a paragraph, and dragging after either extends by that unit; dragging past the
  edge scrolls. The context menu offers Undo, Redo, Cut, Copy, Paste and Select All. Input
  method text being composed is shown underlined in place (the clause being converted thicker)
  without entering the value, the candidate window is placed through `set_cursor_rectangle`
  under it, and a click while composing keeps the text and resets the input method. Dead keys and
  the Compose key work in every window without an input method: the seat follows xkbcommon's
  Compose table of the session's locale (read at the first dead key) and reports the pending accent
  as `Event::Preedit` and the result as ordinary `Key::Character` events, so every field, password
  entry and terminal gets it. As on macOS, a dead key and Space type the accent itself (on a layout with no key of its own for
  `'` or `"`, such as US International, that character instead), a key that
  cannot combine types the accent and then that key (`~` then `/` is `~/`), and Backspace or Escape
  abandon the accent. Shortcuts with Ctrl, Alt or Super never compose. Text is
  published to screen readers as text runs with the caret and selection, which they can also
  set. `TextView` wraps, scrolls with a `ScrollState`, keeps the caret in view and can be
  read-only; revealing the caret lays out the paragraphs it scrolls to and follows again, so a
  far jump in a long wrapped document lands on the caret even though their heights were
  estimates. `TextView::document` is the editor of a document app: no change callback (the app
  compares `revision`), a font (`set_font`: monospace or proportional, any size), a page width
  that wraps to a centred column instead of the view (`set_page_width`), and `insert` and
  `select` that scroll to the result. A tab is laid out as a space widened to the next stop every
  four spaces, so code and logs line up in any font.
  The editor also holds attributed text: each paragraph keeps runs of `TextAttributes` (bold,
  italic, underline, strikethrough, typeface family, size, colour and a link target) and one
  `ParagraphAttributes` (alignment, a bullet or numbered list marker and a nesting level that
  indents it). `TextView::rich_text` and `set_rich_text` move a `RichText` (the plain text with
  attribute ranges and one entry per paragraph) in and out; `change_attributes` restyles the
  selection, or the attributes typing will use at a collapsed caret, and `change_paragraphs`
  restyles the paragraphs it touches, each one undo step. Typing takes the attributes of the
  character before the caret, a new paragraph continues its list, and undo restores removed text
  with its attributes. List markers are drawn in the indent, numbered per level, and are not part
  of the text; underline and strikethrough are drawn from the font's metrics. A plain document
  keeps no runs, so large plain text costs nothing extra.
- **Animation.** `ui::Animated` holds a value that springs to its target with a preset from
  `bande-spring` (`Spring::QUICK`, `STANDARD`, `GENTLE`, `FADE`, ...). A new target retargets
  from the current position and velocity, so a reversed action never jumps or queues. Values
  are sampled at the frame's time; while any value moves, `Ui::is_animating` is true and an app
  returns it from `App::animating`, which asks for the next frame after each one drawn. With
  Reduce Motion (`reduced-motion` of `org.freedesktop.appearance`, in
  `Appearance::reduce_motion`, applied by `Ui::set_appearance`) every spring but `FADE` jumps
  to its target, so movement is gone and fades stay. Switches slide their knob and blend their
  track; the window controls' backfills fade in.
- **Accessibility.** Every widget produces an AccessKit node: role, label or value, bounds,
  toggled state or numeric range, and its actions. `Ui::accessibility` returns the `TreeUpdate`
  with the focus, and `Ui::perform` carries out Focus, Click, SetValue, Increment, Decrement,
  ReplaceSelectedText and SetTextSelection requests. The tree is published on AT-SPI through `accesskit_unix`;
  table cells carry their text as their name, so a screen reader reads them. `toolkit/tests/accessibility.rs`
  finds and presses a button over AT-SPI in a window on the headless compositor.
- **Input.** `wl_seat` up to version 9. Pointer events carry the button (primary, secondary,
  middle, back, forward), the click count (a press within 500 ms and 4 pt of the previous press
  of the same button counts up) and the modifiers; Ctrl+primary stays a primary click with Ctrl
  held, which toggles an item in a selection. Scrolling
  arrives as `Event::Scroll` with its source (wheel, finger, continuous) and phase (a finger
  scroll begins, changes and ends at `axis_stop`, so momentum can start from its end).
  Modifier-only changes arrive as `Event::ModifiersChanged`. `Ui` sends `PointerEnter` and
  `PointerLeave` to every widget on the hit path, bubbles scrolling from the innermost widget
  outwards, and reports the hovered widget's cursor, which `Window::set_cursor` shows through
  `wp_cursor_shape_v1`. A close request (close button, Ctrl+W in the app, Alt+F4) asks `App::should_close`, so an
  app can keep its window open, ask first or close a tab instead; `Window::close` closes it.
- **External surfaces.** Pixels an app gets from elsewhere (a VM's display, later video) go
  to the compositor without passing through the app's scene. `Window::create_external_surface`
  makes an `ExternalSurface`: a desync `wl_subsurface` with a `wp_viewport` and an empty input
  region. `show_shm(fd, ShmLayout)` wraps the app's own memory in a `wl_shm` buffer without a
  copy, `show_dmabuf(&Dmabuf)` imports a dmabuf through `zwp_linux_dmabuf_v1` (`flipped` for
  bottom-up GL scanouts), and `damage(&[Region])` commits the changed rectangles of the current
  buffer. For a pool of buffers (video), `shm_buffer` and `dmabuf_buffer` make a reusable
  `SurfaceBuffer` with a callback for `wl_buffer.release`, shown with `present`.
  `set_idle_inhibited` holds a `zwp_idle_inhibitor_v1` on the surface for as long as it lives. A
  frame wakes neither the app's scene nor the window's renderer.
  `ui::SurfaceView::new(surface.id(), label, on_input)` places it: it paints an
  `Item::Surface`, which the renderers draw as black letterboxing around a transparent hole
  (cut through the enclosing clips, so it keeps the sheet's rounded corners). The window puts
  the subsurface below itself under the hole (above it when its GPU surface is opaque), leaves
  the hole out of its opaque region, and positions and sizes it on the frame that moved it, so
  sheets, tooltips and the capture hint draw over the pixels. `SurfaceScaling::Fit` keeps the
  aspect ratio, `Native` shows one buffer pixel per device pixel (shrunk only when it does not
  fit); `set_size(width, height)` gives the buffer's size.
- **Input pass-through and capture.** `SurfaceView` sends `SurfaceInput` to `on_input`: the
  pointer in buffer pixels, buttons, scrolling, and while captured raw evdev key codes and
  relative motion. With `.capture_on_click(PointerCapture::{Free, Confined, Locked})` a click
  captures: the window inhibits the compositor's shortcuts (`zwp_keyboard_shortcuts_inhibit_v1`,
  so Super and Alt+Tab reach the guest), confines or locks the pointer
  (`zwp_pointer_constraints_v1`, with `zwp_relative_pointer_v1` motion while locked, the cursor
  hidden), and delivers every key as `Event::Captured(Captured::Key)` instead of text. As in
  macOS VM apps, Ctrl+Alt (`RELEASE_CHORD`) releases, and a hint "Press Ctrl+Alt to release"
  shows for 3 s; leaving the window, losing the lock or a sheet appearing releases too, and held
  keys are sent up first. Widgets ask through `Context::capture_input`/`release_input`;
  `Window::capture_input`/`release_input` do it for apps without `Ui`.

Since `wayland-backend` is built with `client_system`, every Wayland client in a workspace
build, `bande-test-wayland` included, goes through libwayland.
