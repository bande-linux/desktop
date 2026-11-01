# Datum: a design language for Bande

Status: adopted by the owner on 2026-10-30 and built; the running desktop is the reference
render. [`datum/src/colors.py`](datum/src/colors.py) keeps the OKLCH and contrast maths behind the
tables below.

> **Datum**: a fixed reference point that everything else is measured from.
> **Quiet surfaces. One signal. Exact edges.**

Bande should feel like a well-made instrument: a Braun radio or a Teenage Engineering synth rather than a pane of glass. Surfaces are solid, warm and calm. Exactly one colour, the *signal*, marks what is selected, focused or live. Edges, radii and spacing follow a few strict rules that anyone can check with a ruler. The smoothness stays macOS-level. The look is our own.

---

## 3. Principles (and how they differ from macOS)

1. **Solid, not glass.** Windows, the bar and menus are opaque, warm surfaces. Depth comes from *tone* (frame vs sheet) and one shadow. macOS (and Liquid Glass) builds identity on translucency; we build it on material honesty, legibility and speed. Blur appears only behind modal moments (§6).
2. **One signal.** Exactly one chromatic colour carries meaning at a time: the accent marks *selected, focused, active, live*. Everything else is warm greys. Semantic red, amber and green appear only for status. App icons are the only place several hues meet.
3. **Frame and sheet.** Every window is a *frame* (the chassis: title bar, toolbar, sidebar) holding an inset *sheet* (the content), whose radius is concentric with the window. You always see where the controls end and your work begins. macOS draws a flat white window with a translucent sidebar instead.
4. **Exact edges.** Radii come from one concentric rule, spacing from a 4 pt scale, and every hairline lands on a device pixel. Nothing is "about 7".
5. **Visible controls.** Glyphs on window controls are always visible. Control boundaries meet 3:1 in Increase Contrast and are never below 1.9:1 by default. A field always looks like a field. macOS hides glyphs until hover and uses very faint outlines.
6. **Labelled like an instrument.** Section labels are small tracked caps, numbers are tabular, and the clock and sizes line up. A square motif (the mark, the running indicator, the cursor in icons) is our dot-matrix: used sparingly.
7. **Precise motion.** The existing springs, critically damped, with movement along straight axes from a visible source. Bounce only when a gesture has momentum. No genie, no wobble.

---

## 4. Geometry

### Radii: five values and one rule
| Token | pt | Used for |
|---|---|---|
| `radius.xs` | 4 | checkbox, tags, minimum for any concentric child |
| `radius.s` | 6 | buttons, fields, pop-ups, steppers, list and menu selection, tooltips |
| `radius.m` | 8 | the content sheet inside a window, segmented track, search field, sidebar groups |
| `radius.l` | 10 | menus, popovers, the hover label |
| `radius.xl` | 14 | windows, alerts, notifications, launcher, bar panels (Control Center, calendar) |
| `radius.full` | h/2 | **only** switch, slider track, badge, progress segments (1) |

**Concentric rule:** `inner = max(outer − inset, 4)`. Examples: window 14 − sheet inset 6 = sheet 8; menu 10 − item inset 4 = item 6; segmented track 8 − 2 = pill 6. A child that stands alone (a button in an alert, 16 from the edge) falls back to its fixed token (6). This is Apple's own "concentric shape" concept, but our values are smaller and our corners are plain circular arcs, **not continuous squircles**. Arcs read crisper and more technical and are cheaper to draw.

The compositor's window corners are 14. Maximised and tiled windows square their screen-touching corners (radius 0), and the sheet keeps its 8.

### Hairlines
- One hairline width: 1 device pixel at integer scales, snapped to the pixel grid at fractional scales (`width = 1/scale` logical, rounded to whole device pixels ≥ 1).
- Allowed in five places only: window outer edge (`edge` at 55 % opacity, inside the shadow), sheet edge in dark mode (`hairline`), table header underline and status-bar top, menu separators, and field outlines (`edge`, 1 pt).
- Never between sidebar rows or settings rows. Use spacing and tone. Never under the title bar: the frame/sheet step replaces it.

### Spacing, density, grid
- Scale (pt): **2, 4, 8, 12, 16, 24, 32, 48**. The existing `Spacing` gets `XXXL = 48`.
- Control heights: **28 regular** (buttons, fields, pop-ups, segmented), 22 small, 36 large (alert and sheet buttons on touch-friendly scale).
- Row heights: list and table 32 (selection 30 inset 1); sidebar 28 with 2 gaps (pitch 30); menu item 28; settings row 44.
- Title bar / unified toolbar ("header"): **48** (was 28). It holds the window controls, navigation, title and toolbar items in one row. Header-less utility windows use 36.
- Sheet inset from the frame: 6 on the right and bottom, 0 at the top (it meets the header), sidebar width + 0 on the left.
- Layout grid: 4 pt base and 8 pt rhythm. Text is flush left everywhere, including the window title, which sits after the navigation instead of being centred.

---

## 5. Colour

Defined in **OKLCH** and generated, Linear-style, from a few inputs: a neutral hue per scheme (warm grey h = 80° in light, dusk slate h = 262° in dark), accent, and contrast. The values below are the generated defaults (sRGB hex).

### Neutrals
| Role | Light | Dark | Use |
|---|---|---|---|
| `chassis` | `#ECEAE7` (L .938) | `#1F2329` (L .255) | window frame, sidebar, title bar, combined bar |
| `sheet` | `#FCFBF9` (L .988) | `#272A30` (L .285) | content inside windows, Settings content, document background |
| `overlay` | `#FFFFFF` | `#34373C` (L .335) | menus, popovers, notifications, alert body |
| `well` | `#E2DFDB` (L .905) | `#31343A` (L .325) | control fill (buttons, segmented track, window-control well, search), unfocused selection, sidebar selection |
| `hairline` | `#D4D0CB` | `#3B3F45` (L .365) | separators, sidebar selection with keyboard focus |
| `rim` | (= hairline) | `#494D54` (L .42) | the 1 pt edge of floating dark surfaces (sheet, menus, notifications, launcher, Control Center, labels); none in light |
| `edge` | `#BAB7B2` | `#575B62` (L .47) | window and control outlines; raised chip in dark segmented |
| `ink` | `#201E1A` | `#E4E6EA` (L .925) | primary text and glyphs |
| `ink2` | `#5B5752` | `#B9BEC6` | secondary text, inactive glyphs |
| `ink3` | `#83807A` | `#8D9198` | labels, placeholders, disabled, timestamps |
| `deep` | `#201E1A` | `#161A21` (L .215) | dark text on light fills (Amber, Graphite, warning), the terminal's Midnight background |

Light keeps the warm hue (paper and stone). Dark moved in 2026 from warm near-black (`#141210`) to a lifted "dusk slate": hue 262, between Breeze/GitHub (248–258) and GNOME (286) and next to the Ultramarine accent, with chroma .011–.013 so the tint shows without reading as blue. The base sits at L .255 like every current dark desktop, not near black. In dark, elevated is lighter: chassis < sheet < well ≤ overlay < hairline < rim < edge. Whether light moves to the same slate family is an open owner decision (plan `NOTES/plans/dark-theme.md`, milestone 6).

### Measured contrast (WCAG 2 ratio / APCA Lc)
| Pair | Light | Dark |
|---|---|---|
| ink on sheet | 16.1 / 101 | 11.5 / −88 |
| ink on overlay | | 9.6 / −85 |
| ink2 on sheet | 6.9 / 83 | 7.7 / −63 |
| ink3 on sheet | 3.8 / 64 | 4.5 / −39 |
| ink2 on chassis | 6.0 / 73 | 8.4 / −65 |
| accent fill vs sheet | 5.4 | 3.1 |
| accent text tone vs overlay | | 4.35 |
| on-accent text on accent | 5.6 (white) | 4.7 (white) |

Targets: body text Lc ≥ 90 in light; in dark Lc 75–90, because APCA gives Lc 90 as the advisory maximum in dark mode, where halation, not lack of contrast, is the problem (a test fails if ink is brightened back to near white); secondary Lc ≥ 60 at 12–13 pt medium (met); `ink3` only for labels ≥ 10.5 pt semibold, placeholders and disabled (Lc ≥ 30 floor; dark `ink3` at Lc 39 is placeholder-only, and labels on dark use `ink2` when ≤ 11 pt in Increase Contrast). Non-text control boundaries: ≥ 1.9:1 default, ≥ 3:1 with Increase Contrast (`edge` darkens to L .62 light / lightens to L .58 dark). Add a test that computes these from the tokens and fails CI below target.

### The signal: accent strategy
- **Default accent: Ultramarine.** Light `#4859DD` (oklch .53 .20 272), dark `#7E96FB` (oklch .70 .15 272). It is bluer-violet than Apple's azure `#007AFF` and sits with the existing blue-violet wallpapers. It is chosen because selection must read as neutral "this one", and a warm signal would read as a warning.
- **Dark selections are mid-tone fills with white text.** Each accent has a dark *fill* tone, solved to luminance ≈ 0.175 so white text reaches ≥ 4.5:1 and the fill stays ≥ 3:1 on chassis and sheet; the brighter tone is the dark *text* tone (links, focus rings, carets, chart lines, ANSI colours). This halves the brightest area on a dark screen. Amber and Graphite keep a light fill with `deep` text, since white cannot reach 4.5:1 on them.
- **User accents** (8), all generated at matched lightness so each passes the same contrast tests:

| Name | Light | Dark text tone | Dark fill |
|---|---|---|---|
| Ultramarine (default) | `#4859DD` | `#7E96FB` | `#576CD4` |
| Teal | `#008384` | `#3FC0C0` | `#008282` |
| Moss | `#48823B` | `#80C271` | `#48823B` |
| Amber | `#BE7100` | `#F7AB5D` | `#E7963D` |
| Ember | `#D24100` | `#F77C56` | `#C94A1F` |
| Rose | `#C83065` | `#F47C9B` | `#C7436D` |
| Plum | `#9043A1` | `#C889D7` | `#9F58B0` |
| Graphite (monochrome) | `#302D28` | `#DADEE5` | `#CDD1D8` |

  `on_accent` is computed (white or `deep`, whichever has the higher contrast, ≥ 4.5:1). For Amber, light mode this yields ink text. An optional "accent from wallpaper" maps the wallpaper's dominant hue onto the nearest preset, so the theme never ends up with an untested colour.
- **Derived accent roles:** `accent` (fills; the dark fill tone in dark), `accent_text` (the text tone, for links, quiet buttons, carets, rings and chart lines), `on_accent`, `accent_soft` (the text tone mixed 84 % / 78 % toward sheet, for drag-over and find highlights), `focus` (= `accent_text`).
- **Semantic** (fixed, not affected by accent): danger `#D42F34` / `#F66D67` (dark text tone `danger_text`, dark fill `#CF4040` for buttons and badges); warning `#D98B09` / `#F7AC4D`; success `#2D8949` / `#62C37A`. Badges use danger. **Ember** is the only brand hue outside the accent: it is the "signal square" inside app icons and nowhere in the UI chrome.

---

## 6. Materials and depth

| Level | Surface | Shadow (light) | Shadow (dark) |
|---|---|---|---|
| 0 Desktop | wallpaper | none | none |
| 1 Frame | `chassis` + 1 pt `edge` at 55 % | key 0/1/2 α .10 + ambient 0/12/32 α .22 (active); ambient halves when inactive | key α .40 + ambient 0/14/36 α .45 |
| 1 Sheet | `sheet`, inset in the frame | none (tonal step only) | none, plus a `rim` edge |
| 2 Overlay | `overlay` (menus, popovers, notifications), lighter than the window in dark | 0/1/1.5 α .14 + 0/6/18 α .18 | 0/1/1.5 α .35 + 0/6/18 α .40, plus a `rim` edge |
| 3 Modal | alert/sheet card above a **scrim**: parent dims with black at 18 % (light) / 40 % (dark) | as frame | as frame |

**Translucency policy:** none on windows, menus or the bar by default. Blur exists in exactly two places: (a) the scrim behind the launcher, overview and modal alerts gets a 24 pt backdrop blur at 60 % mix, so the eye is pulled forward; (b) Reduce Transparency is irrelevant because we are opaque, but an opt-in "Tinted chassis" mixes 6 % of the wallpaper's average colour into `chassis` (Mica-like, computed once per wallpaper change, no live blur). This also drops the per-frame blur work the Dock and menus do today.

**Inactive windows:** the header glyphs go to `ink3`, the window-control well loses its fill and becomes an outline, the selection turns from accent to `well`, the ambient shadow halves, and the sheet stays the same, so content stays readable.

---

## 7. Typography

**UI text: Inter** (already installed, `fonts-inter`, SIL OFL), set apart from SF with Inter 4's own features:
`cv05` (lowercase l with tail: disambiguates Il1 in file names), `cv08` (capital I with serifs), `ss07` and `ss08` (square punctuation and quotes: the square motif at the size of a full stop), and `tnum` wherever numbers are in columns (tables, clock, sizes, sliders). parley already sets `FontFeatures`, so this is a one-line change in `text.rs`.

**Display and voice: Manrope** (`fonts-manrope`, Debian main, SIL OFL; weights 200–800). It is a modern geometric grotesk with open apertures and a slightly wide stance, clearly not SF Pro Display. It is used from 15 pt up: window titles, page titles, alert headings and the lock screen. It is what makes a screenshot recognisable as Bande.

**Mono: JetBrains Mono** (`fonts-jetbrains-mono`, OFL; with `zero` slashed) replaces DejaVu Sans Mono in the terminal and code.

Considered and not chosen: Geist (not packaged in Debian, and now Nothing's and Vercel's identity); IBM Plex (not in Debian trixie, and corporate in tone); Atkinson Hyperlegible (only Regular and Bold in Debian; we propose it as the *Larger Text / Hyperlegible* accessibility option); Figtree (not packaged).

| Style | Face | Size / line | Weight | Notes |
|---|---|---|---|---|
| Display | Manrope | 28 / 34 | 700 | Settings hero, setup, lock |
| Title | Manrope | 22 / 28 | 700 | page headings |
| Heading | Manrope | 17 / 22 | 700 | alerts, sheets, group titles |
| Window title | Manrope | 15 / 20 | 700 | flush-left in header |
| Body | Inter | 13 / 18 | 400 | default |
| Body strong | Inter | 13 / 18 | 600 | names, emphasis |
| Callout | Inter | 12 / 16 | 400 | status, secondary lines |
| Label | Inter | 10.5 / 14 | 600, CAPS, +0.07 em | section headers, column headers, notification app name |
| Mono | JetBrains Mono | 13 / 18 | 400 | terminal, code |
| Clock (bar) | Inter | 15 / 18 | 600 tnum | stacked HH / MM in the collapsed bar |

The lock-screen clock is Manrope 300 at 96 pt with `tnum`.

---

## 8. Window chrome

**Header (48 pt, `chassis`)**: from the left: window controls, 16 gap, back/forward (when relevant), the **title flush left** in Manrope 15/700, toolbar items, then search on the right (`well`, radius 8). No separator line: the sheet's top edge is the division.

**Window controls (left, ours, not traffic lights)**: one **well**, a 66×24 pill with radius 12 (`well` fill), holding three neutral glyphs on a 21 pt pitch:
- **Close**: an × with 7 pt arms and a 1.5 stroke, round caps.
- **Minimise**: an 8 pt bar.
- **Zoom**: two opposing corner brackets (expand). It changes to inward brackets while zoomed or full screen, so the glyph tells the state.
- **Rest**: glyphs `ink2`, **always visible** (honest, and discoverable on first use).
- **Hover over the well**: all glyphs go to `ink`. The hovered *close* gets a 16 pt danger disc with a white ×. Zoom and minimise get no colour (one signal: only the destructive action turns red).
- **Press**: a disc of `ink` at 12 % behind the glyph, plus the `quick` spring.
- **Inactive window**: the well becomes a 1 pt `hairline` outline and the glyphs go to `ink3`.
- **Alt held**: zoom shows a "+" (zoom to fit) instead of brackets, following DESIGN §1's progressive disclosure.
- The hit target is the full 24×21 cell; the whole well area to the screen corner is clickable when maximised (Fitts).

**Sidebar**: on `chassis` (part of the frame). It uses tracked labels ("PLACES"), 16 pt line symbols in `ink2`, a 28 pt row and a tone-only selection with radius 6: `well` in the active window, `hairline` while the sidebar has the keyboard, and `well` halfway to `chassis` in an inactive window. The selected symbol turns `ink`; sidebar rows get no accent and no focus ring. Content selection (on the sheet) is accent-filled, so "where I am" (grey) and "what I picked" (signal) are never confused.

**Sheet**: content on `sheet`, radius 8, inset 6.

**Focus**: a 2 pt `focus` ring at a 1 pt gap, with radius = control radius + 3 (concentric outward). Text fields swap their outline to accent 1.5 pt and add the ring at 35 % opacity. The ring animates with `quick`.

**Server-side decorations** for third-party apps draw the same header, at 40 pt instead of 48 because there is no toolbar, from the same `title_bar.rs`.

---

## 9. Controls

| Control | Spec |
|---|---|
| Primary button | accent fill, `on_accent` 13/600, radius 6, 28 high, no gradient, no outline; press mixes 15 % black |
| Secondary button | `well` fill, `ink` 13/500; hover mixes 8 % ink |
| Quiet button | text in `accent` 600, fill appears on hover only |
| Destructive | danger fill, white (light) or `chassis` (dark) text; only for the actual destructive verb |
| Text field | `overlay` (light) / `well` (dark) fill, 1 pt `edge` outline, radius 6; focus = accent outline + ring; caret 1.5 pt accent |
| Search field | `well` fill, no outline, radius 8, magnifier in `ink3` |
| Switch | 36×20 pill; on = accent, off = `edge`; white 15 pt knob with the small shadow |
| Checkbox | 16 pt, radius 4; off = 1.5 pt `ink3` outline; on = accent with a **square-capped** check |
| Radio | 16 pt circle; on = accent with a **5 pt square** centre (the square motif), not a dot |
| Segmented | `well` track radius 8, raised chip radius 6 (`overlay` in light, `edge` in dark) with the small shadow; selected label 600 `ink` |
| Pop-up | `well` fill, single down-chevron (not the ⌃⌄ pair) |
| Stepper | one 96 pt `well` capsule: − value + (value in tnum) |
| Slider | 4 pt track (`well`), accent fill, and a **fader knob**: a 7×20 rounded bar, white with an edge. It reads as an instrument fader, not an iOS ball. Optional tick marks 1.5×4 `ink3` |
| Progress | **segmented**: 24 cells of 2 gap, radius 1, accent on `well`. Indeterminate = a 4-cell block gliding (the `standard` spring) |
| Menus | `overlay`, radius 10, 4 inset, 28 rows, highlight = accent radius 6; shortcut column right-aligned in `ink3`, written as words in GNOME order ("Shift+Ctrl+N", "Alt+Super+8"; no ⌘, ⌃, ⌥ or ⇧ symbols); submenus open on the `quick` spring; a menu never leaves the screen: it flips, then slides, then shortens to scroll between 20 pt bands with a single `ink2` chevron (behaviour: `docs/architecture/behaviour.md`, Popup placement) |
| Alerts | card radius 14: the body on `overlay` with a 36 pt app disc, a Manrope 17 heading and body in `ink2`; a **footer tray** on `chassis` holding the buttons (default right, cancel left of it, an alternative such as "Don't Quit" as a quiet button on the far left). App-modal alerts are centred over the window with a scrim; document-modal alerts drop from under the header |
| Tooltip | `ink` fill with `chassis` text (inverted), radius 6, 12/500; no arrow |
| Notifications | `overlay`, radius 14, app disc 18 + LABEL-style app name + time, title 13.5/600, body 13 `ink2`; actions appear as quiet buttons on hover |
| Badges | danger capsule, 11/700 white, min 18 pt |

---

## 10. Iconography

**App icons** are rounded squares, as in the bar mockup the owner picked: on a 48 grid a 44 pt
body with circular-arc corners of radius 10 (not a squircle), a rich but clean colour field (a
subtle vertical gradient is allowed) and a detailed, well-crafted symbol with depth through tone,
not gloss. Third-party icons are shown unmodified.

**File icons: "Sheet + label."** A 30×40 white sheet, radius 3, with a hairline edge and **no dog-ear**, a content hint on top (text lines, picture, zipper) and a **coloured label band** at the bottom with the extension in JetBrains Mono 700 (PDF red, PNG moss, TXT warm grey, ZIP amber, audio plum, video rose, code teal, unknown `ink3`). Thumbnails replace the hint area for images and PDFs, and the label band stays.

**Folders**: a stone body (`#D9D3CA`) with an **accent tab**, so folders follow the user's accent while the folder itself stays neutral. Special folders have an engraved `ink2` symbol on the body (download arrow, picture, music, home).

**Symbols** (sidebar, menus, bar status, tray, Trash, view modes): a 16/18 pt grid, drawn with the detail and weight of the bar mockup and filled where it helps legibility, recoloured with the text colour; accent only when selected. They are an SVG set in `data/icons/Bande/symbolic`.

**The theme** (`data/icons/Bande`) is generated by `python3 data/icons/src/generate.py`, which needs `fonts-jetbrains-mono` for the label bands (outlines are baked into the SVGs) and also writes our apps' hicolor icons. Symbolic icons are 16 pt, 1.6 strokes converted to single-colour fills (`#201E1A`, with `opacity` for unlit parts and `class="warning"`/`"error"` for low battery), so GTK, Qt and our toolkit can recolour them with the text colour. The accent tab of folders comes from hidden sibling themes `Bande-<Accent>` (`Inherits=Bande`) that hold only the recoloured folders: the session selects the icon theme `Bande-Teal` and so on for a non-default accent.

---

## 11. Motion signature

The presets are `bande-spring`'s (`QUICK`, `STANDARD`, `GENTLE`, `FADE`, `INTERACTIVE`, `MOMENTUM`, `BOUNCY`, `SHAKE`). Datum adds **how** things move, Material 3-style: *spatial* springs for position and size, *effects* springs (`fade`) for colour and opacity, never mixed.

- **Axis-true:** objects travel in straight lines from their source. Minimise slides and scales into its exact bar slot along one straight path (`gentle`), with no genie distortion. Restore reverses it.
- **Open / close:** the window scales 0.96 → 1 and fades in from the bar icon's side (`standard`); close is 1 → 0.98 + fade (`quick`).
- **The selection block glides:** in lists, menus, segmented controls and the sidebar, the highlight *moves* to the new row (`quick`) rather than blinking. This is the most visible "Datum" motion.
- **The signal square blinks only once:** running and launch feedback pulses the square indicator (opacity 1 → .35 → 1 on `fade`) until the first window maps; no bouncing icons.
- **The fader knob** follows 1:1 and settles on detents with `interactive`.
- **Bounce** (`momentum`, 0.6 % overshoot) only after a flick. `bouncy` is reserved for the attention pulse of a bar icon.
- Reduce Motion: every spatial spring becomes `fade`, per the existing rule.

## 12. Sound and haptics (brief)

A dry, short, *wooden-percussive* palette: soft-attack mallet and click tones under 150 ms, all pitched within D major pentatonic so overlapping sounds never clash, mastered to about −20 LUFS. Alert is two notes (A4→D5); trash is a soft paper rustle plus a low tick; screenshot is a mechanical shutter click; volume tick is a single 5 ms woodblock; plug-in rises a fifth, unplug falls a fifth. There are no chimes and no glassy pads, the sonic equivalent of solid material. Haptics on trackpads that support them: one light tick on slider detents and on snapping a window to a tile.

## 13. The shell: the combined bar

The combined bar (the Taskbar) is an edge-anchored bar, 64 pt at the default icon size, on the left holding: the mark, the app menu block, apps, Trash, desktops, a status cluster and the clock. Datum fits it directly, and fixes the parts of those mockups that still read as macOS (traffic lights, squircle icons, blue folders, a gear):
- Bar surface = `chassis` (same tone as window frames, so the bar reads as the desktop's frame), with a 1 pt `edge` on the inner side only, square at the screen edge. It is opaque; "Tinted chassis" applies here too.
- **Active app = a 3×16 accent tick** on the screen-edge side; **running = a 3×3 `ink2` square** (the square motif; a dot would be macOS).
- The app-menu block and the status cluster are `well` groups with radius 8 (concentric with 14 panels).
- The clock is Inter 15/600 tnum stacked "14 / 32".
- The mark is a "U" of seven 4.5 pt squares in `ink`.
- Bar pop-overs (Control Center, calendar, notifications) are `overlay` panels with radius 14 that slide out from the bar along its normal axis (`standard`).
- Horizontal placement uses the same rules with the tick on the bottom edge.
