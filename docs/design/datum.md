# Datum: a design language for Bande

Status: adopted by the owner on 2026-10-30. Implementation order: `docs/roadmap.md`, wave D.
Mockups: `datum/` (SVG and PNG, rendered with resvg 0.49 from the workspace's own dependency). The generator scripts are in `datum/src/`, so every value below is the value used in the pictures.

> **Datum**: a fixed reference point that everything else is measured from.
> **Quiet surfaces. One signal. Exact edges.**

Bande should feel like a well-made instrument: a Braun radio or a Teenage Engineering synth rather than a pane of glass. Surfaces are solid, warm and calm. Exactly one colour, the *signal*, marks what is selected, focused or live. Edges, radii and spacing follow a few strict rules that anyone can check with a ruler. The smoothness stays macOS-level. The look is our own.

---

## 1. Today's look: honest critique

Based on the goldens under `crates/**/tests/golden/`.

### Reads as a macOS copy

| Element | Where | Why it reads as macOS |
|---|---|---|
| Traffic lights | `graphics/src/title_bar.rs` (`#FF5F57`, `#FEBC2E`, `#28C840`, 12 pt discs, 20 pt pitch, ×/−/+ glyphs on hover) | These are Apple's exact hex values and geometry. This is the single strongest "clone" signal. |
| Centred 13 pt semibold title on a white 28 pt bar with a hairline under it | `title_bar.rs`, every window golden | The classic AppKit title bar. |
| Menu bar layout | session shots: logo glyph, **bold app name**, File Edit View Go Window, then status icons and "Fri 30 Oct, 05:57" on the right | Copied item for item. The combined bar replaces it. |
| Inter at 13 pt with SF-like metrics, `Inter Display` for large sizes | `graphics/src/text.rs` | Inter is the closest free face to SF Pro, so it reads as "SF but not quite". |
| Accent `#2F6FEB` / `#5B8DEF` | `tokens.rs` | Apple-system-blue territory. |
| Big Sur squircle tiles with a gradient fill and a white letter, plus the gear Settings icon | `dock/src/lib.rs` (`#8E9AAF`→`#5C677D` fallback tile), Dock goldens | The macOS app icon grammar. |
| Blue folders, pages with a dog-ear | Files, desktop goldens, `data/icons/Bande/scalable/places` | Finder's folder colour and the paper metaphor. |
| Translucent Dock and menus (`F7F8FA` at alpha 170 over blur) | `dock/src/lib.rs:1596-1609`, menu golden | Vibrancy. |
| Alert layout (bold title, grey body, Cancel + blue default button bottom-right, window chrome around the alert) | `toolkit/platform/tests/golden/alert.png` | Matches NSAlert. Its title bar even reads "Documents", which is a bug. |
| Segmented control with a raised white pill, the pop-up button with ⌃⌄ chevrons, and the ⌘ glyph in menus | controls and menu goldens | AppKit controls redrawn, down to the ⌘ symbol. |
| Settings: grouped inset cards, checkmark rows, "MacBook Pro Speakers" | `settings/tests/golden/sound-light.png` | System Settings (Ventura and later). |
| Spotlight-style launcher ("Top Hit", a preview pane, "Return to open") | launcher goldens | Spotlight's layout and vocabulary. |

### Feels half-done

- **Radii have no system.** Count today: checkbox 3.5, button, field and pop-up 5, in-place field 4, search field 7, segmented 7 with a 5 pill, rows 7, menu 8, sheet 10, compositor windows 12, notification 16, launcher card 16. Nothing is concentric. A 5 pt button inside a 12 pt window inside a 16 pt card reads as accidental.
- **Colour is hard-coded in about 90 places** (`grep hex(0x` finds 24 in the Dock alone, plus the terminal, Files, monitor, menus and sheets). `Palette` has only six roles, so each surface invents its own greys: `#2A2D33` in the Dock, `#17191D` as the dark background, `#F4F5F7` light.
- **Too little contrast in chrome.** Field and control outlines are around 1.3:1. The unfocused window's grey traffic lights vanish. The dark launcher is a flat slab whose result icons are empty grey squares.
- **Placeholder art is visible.** The Dock shows an "E" letter tile and a blank tile. Desktop folders are flat blue squares and file thumbnails are blank grey squares (desktop goldens). Our own Files/Terminal/Settings icons sit next to Adwaita mimetype icons.
- **Flat, unconsidered areas.** The Files columns pane leaves a huge empty grey preview area. The Settings sidebar is pure white against a grey content area, the opposite tonal logic from the windows. The alert's dimmed parent shows through at the bottom (golden).
- **Mixed signals for the same state.** Selection is a blue fill in tables, a grey fill in the Files sidebar and a blue fill in Settings. The focus ring of the Settings search field is a thick, glowing light-blue halo.
- **The monospace face** (DejaVu Sans Mono) has a different character from the UI face and looks dated next to Inter.

---

## 2. What the other languages teach (principles, not looks)

| Language | What makes it distinctive | Principle we take | Source |
|---|---|---|---|
| **Apple Liquid Glass (2025)** | A dynamic glass layer *above* content, controls morph, and shapes are concentric with the hardware: fixed radii, capsules, or "concentric" = parent radius − padding | **Concentricity as a computed rule.** We reject glass as the default material: it is now Apple's signature and it costs legibility. | [WWDC25 session 356](https://developer.apple.com/videos/play/wwdc2025/356/), [wwdcnotes summary](https://wwdcnotes.com/documentation/wwdc25-356-get-to-know-the-new-design-system/), [critique: pxlnv](https://pxlnv.com/blog/on-liquid-glass/) |
| **Material 3 Expressive** | Springs instead of easing curves, split into *spatial* springs (movement) and *effects* springs (colour and opacity); a large shape library; bold type contrast | Separate spatial and effects motion; let emphasis come from type contrast. We avoid shape whimsy. | [pub.dev material_expressive](https://pub.dev/packages/material_expressive) (implementation summary; Google's primary M3 Expressive page not fetched, unverified) |
| **Fluent 2 / Windows 11** | Mica: an *opaque* material lightly tinted by the wallpaper on long-lived surfaces; Acrylic (blur) only on transient flyouts; layering by tone | **Long-lived surfaces opaque, transient ones may differ; depth by tonal layers.** | [Fluent Design System](https://en.wikipedia.org/wiki/Fluent_Design_System), [Microsoft Learn: Mica](https://learn.microsoft.com/windows/apps/design/style/mica) |
| **GNOME libadwaita** | Few well-named colour roles (`accent_bg`, `accent_fg`, standalone `accent`); a fixed set of accents since 1.6; a light text colour is swapped for dark when the accent is too bright | **Accent split into bg / fg / standalone roles, with the foreground computed automatically.** | [libadwaita AccentColor](https://gnome.pages.gitlab.gnome.org/libadwaita/doc/1.6/enum.AccentColor.html), [styles & appearance](https://gnome.pages.gitlab.gnome.org/libadwaita/doc/1.5/styles-and-appearance.html) |
| **KDE Breeze → Union / Ocean** | One CSS-based style engine for every toolkit, driven by a design system (Ocean) | **One token source drives every renderer** (our toolkit, compositor-drawn title bars, GTK/Qt via portal). | [Union FAQ](https://discuss.kde.org/t/get-started-with-union-plasmas-new-theming-engine-in-2-minutes-or-less-a-faq/47883), [KDE goal](https://invent.kde.org/teams/goals/goal-setting/2026/-/work_items/3) |
| **COSMIC (System76)** | Rust/Wayland; themes generated from a few inputs; roundness levels; frosted glass added in Epoch 1.3 as an opt-in | **Generate the theme from a few inputs.** Translucency is a user option, not the identity. | [Phoronix: Epoch 1.3](https://phoronix.com/news/COSMIC-Epoch-1.3) |
| **elementary OS** | Craft and restraint; Inter as UI type; layered stylesheets | Craft in small details; one primary action per view. | [elementary stylesheet](https://github.com/elementary/stylesheet) |
| **ChromeOS** | A shelf with a launcher button at its start and status at its end, colours derived from the wallpaper | A shell bar can be calm and solid; status is grouped into one quiet cluster. | (general knowledge, unverified) |
| **Nothing OS** | Monochrome, dot-matrix used *sparingly* "where it can define character rather than compromise readability"; Geist for body text; red as the only signal | **A single signal colour; motif used sparingly; a readable body face.** | [Nothing OS evolution](https://nothing.community/d/61774-nothing-os-evolution) |
| **Teenage Engineering** | Constraints as aesthetic; colour codes function; monospaced labels; instrument panels | **Labels as instrument markings; colour carries meaning, never decoration.** | [Blake Crosley: TE guide](https://blakecrosley.com/guides/design/teenage-engineering) |
| **Linear / Arc / Raycast** | Linear generates a whole theme from base + accent + contrast in LCH; Arc puts content on an inset "card" inside a coloured frame; Raycast is keyboard-first with dense lists | **A perceptual (OKLCH) theme generator with 3 inputs; content as an inset sheet in a frame.** | [Linear: how we redesigned the UI](https://linear.app/now/how-we-redesigned-the-linear-ui) |
| **Vercel Geist** | High-contrast greyscale, a near-black `#171717`, and blue as the only accent; Geist Sans/Mono under the OFL | Greys do the work and the accent is rare. | [vercel.com/geist](https://vercel.com/geist), [design.md](https://vercel.com/design.md) |
| **Dieter Rams** | Ten principles: useful, understandable, unobtrusive, honest, thorough to the last detail, as little design as possible | **Honest, unobtrusive, thorough.** Controls look like what they do; glyphs are always visible. | [Vitsœ: good design](https://vitsoe.com/us/about/good-design) |
| **Swiss typography** | A grid, flush-left ragged-right text, a hierarchy built from weight and size, small tracked caps for labels | Flush-left titles, a 4/8 grid, tracked small-caps labels. | (canon: Müller-Brockmann, *Grid Systems*) |
| **APCA** | Perceptual contrast: body text Lc 75 minimum, Lc 90 preferred; Lc 60 for large text; Lc 45 large fluent; Lc 30 for placeholder text; Lc 15 non-text | Contrast targets below. | [APCA introduction](https://github.com/Myndex/apca-introduction), [apca-w3](https://github.com/Myndex/apca-w3) |

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

The compositor's `CORNER_RADIUS` 12 becomes 14. Maximised and tiled windows square their screen-touching corners (radius 0), and the sheet keeps its 8.

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

Defined in **OKLCH** and generated, Linear-style, from a few inputs: base hue (warm grey, h = 80°), accent, and contrast. The values below are the generated defaults (sRGB hex, as used in the mockups).

### Neutrals
| Role | Light | Dark | Use |
|---|---|---|---|
| `chassis` | `#ECEAE7` (L .938) | `#141210` (L .185) | window frame, sidebar, title bar, combined bar |
| `sheet` | `#FCFBF9` (L .988) | `#1D1B19` (L .225) | content inside windows, Settings content, document background |
| `overlay` | `#FFFFFF` | `#272522` (L .265) | menus, popovers, notifications, alert body |
| `well` | `#E2DFDB` (L .905) | `#292724` (L .275) | control fill (buttons, segmented track, window-control well, search), unfocused selection |
| `hairline` | `#D4D0CB` | `#32302D` | separators |
| `edge` | `#BAB7B2` | `#44423F` | window and control outlines; raised chip in dark segmented |
| `ink` | `#201E1A` | `#F1F0ED` | primary text and glyphs |
| `ink2` | `#5B5752` | `#BAB7B2` | secondary text, inactive glyphs |
| `ink3` | `#83807A` | `#8F8C87` | labels, placeholders, disabled, timestamps |

The warm hue (paper and stone, not blue-grey) is a deliberate break from macOS's cool neutrals and COSMIC's blue-black. Chroma stays ≤ 0.010, so it reads as "warm neutral", not beige. The light frame is *darker* than the sheet, and the dark frame is *darker* than the sheet. In both schemes the content is the brightest surface: one tonal logic.

### Measured contrast (WCAG 2 ratio / APCA Lc)
| Pair | Light | Dark |
|---|---|---|
| ink on sheet | 16.1 / 101 | 15.1 / −97 |
| ink2 on sheet | 6.9 / 83 | 8.6 / −62 |
| ink3 on sheet | 3.8 / 64 | 5.1 / −39 |
| ink2 on chassis | 6.0 / 73 | 9.4 / −63 |
| accent vs sheet | 5.4 | 6.3 |
| on-accent text on accent | 5.6 (white) | ≈6.5 (chassis ink) |

Targets: body text Lc ≥ 90 (met); secondary Lc ≥ 60 at 12–13 pt medium (met); `ink3` only for labels ≥ 10.5 pt semibold, placeholders and disabled (Lc ≥ 30 floor; dark `ink3` at Lc 39 is placeholder-only, and labels on dark use `ink2` when ≤ 11 pt in Increase Contrast). Non-text control boundaries: ≥ 1.9:1 default, ≥ 3:1 with Increase Contrast (`edge` darkens to L .62 light / L .52 dark). Add a test that computes these from the tokens and fails CI below target.

### The signal: accent strategy
- **Default accent: Ultramarine.** Light `#4859DD` (oklch .53 .20 272), dark `#7E96FB` (oklch .70 .15 272). It is bluer-violet than Apple's azure `#007AFF` and sits with the existing blue-violet wallpapers. It is chosen because selection must read as neutral "this one", and a warm signal would read as a warning.
- **Dark mode inverts the on-accent text:** selection in dark is a light accent fill with **dark** text (`chassis`). This is one of the most recognisable Datum traits (see `window-dark.png`); macOS keeps white-on-blue.
- **User accents** (8), all generated at matched lightness so each passes the same contrast tests:

| Name | Light | Dark |
|---|---|---|
| Ultramarine (default) | `#4859DD` | `#7E96FB` |
| Teal | `#008384` | `#3FC0C0` |
| Moss | `#48823B` | `#80C271` |
| Amber | `#BE7100` | `#F7AB5D` |
| Ember | `#D24100` | `#F77C56` |
| Rose | `#C83065` | `#F47C9B` |
| Plum | `#9043A1` | `#C889D7` |
| Graphite (monochrome) | `#302D28` | `#E1DDD7` |

  `on_accent` is computed (white or `chassis` ink, whichever has the higher contrast, ≥ 4.5:1). For Amber, light mode this yields ink text. An optional "accent from wallpaper" maps the wallpaper's dominant hue onto the nearest preset, so the theme never ends up with an untested colour.
- **Derived accent roles:** `accent` (fills), `accent_text` (= accent, for links and quiet buttons), `on_accent`, `accent_soft` (accent mixed 84 % toward sheet, for drag-over and find highlights), `focus` (= accent).
- **Semantic** (fixed, not affected by accent): danger `#D42F34` / `#F66D67`; warning `#D98B09` / `#F7AC4D`; success `#2D8949` / `#62C37A`. Badges use danger. **Ember** is the only brand hue outside the accent: it is the "signal square" inside app icons and nowhere in the UI chrome.

---

## 6. Materials and depth

| Level | Surface | Shadow (light) | Shadow (dark) |
|---|---|---|---|
| 0 Desktop | wallpaper | none | none |
| 1 Frame | `chassis` + 1 pt `edge` at 55 % | key 0/1/2 α .10 + ambient 0/12/32 α .22 (active); ambient halves when inactive | key α .5 + ambient 0/14/36 α .55 |
| 1 Sheet | `sheet`, inset in the frame | none (tonal step only) | none, plus a `hairline` edge |
| 2 Overlay | `overlay` (menus, popovers, notifications) | 0/1/1.5 α .14 + 0/6/18 α .18 | 0/1/2 α .5 + 0/14/36 α .55 |
| 3 Modal | alert/sheet card above a **scrim**: parent dims with black at 18 % (light) / 40 % (dark) | as frame | as frame |

**Translucency policy:** none on windows, menus or the bar by default. Blur exists in exactly two places: (a) the scrim behind the launcher, overview and modal alerts gets a 24 pt backdrop blur at 60 % mix, so the eye is pulled forward; (b) Reduce Transparency is irrelevant because we are opaque, but an opt-in "Tinted chassis" mixes 6 % of the wallpaper's average colour into `chassis` (Mica-like, computed once per wallpaper change, no live blur). This also drops the per-frame blur work the Dock and menus do today.

**Inactive windows:** the header glyphs go to `ink3`, the window-control well loses its fill and becomes an outline, the selection turns from accent to `well`, the ambient shadow halves, and the sheet stays the same, so content stays readable.

---

## 7. Typography

**UI text: Inter** (already installed, `fonts-inter`, SIL OFL), set apart from SF with Inter 4's own features:
`cv05` (lowercase l with tail: disambiguates Il1 in file names), `cv08` (capital I with serifs), `ss07` and `ss08` (square punctuation and quotes: the square motif at the size of a full stop), and `tnum` wherever numbers are in columns (tables, clock, sizes, sliders). parley already sets `FontFeatures`, so this is a one-line change in `text.rs`.

**Display and voice: Manrope** (`fonts-manrope`, Debian main, SIL OFL; weights 200–800). It is a modern geometric grotesk with open apertures and a slightly wide stance, clearly not SF Pro Display. It is used from 15 pt up: window titles, page titles, alert headings, the lock screen and the expanded bar's clock. It is what makes a screenshot recognisable as Bande.

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

**Sidebar**: on `chassis` (part of the frame). It uses tracked labels ("PLACES"), 16 pt line symbols in `ink2`, a 28 pt row and a `well`-filled selection with radius 6. The selected symbol turns accent. Content selection (on the sheet) is accent-filled, so "where I am" (grey) and "what I picked" (signal) are never confused.

**Sheet**: content on `sheet`, radius 8, inset 6.

**Focus**: a 2 pt `focus` ring at a 1 pt gap, with radius = control radius + 3 (concentric outward). Text fields swap their outline to accent 1.5 pt and add the ring at 35 % opacity. The ring animates with `quick`.

**Server-side decorations** for third-party apps (roadmap A1) draw the same header, at 40 pt instead of 48 because there is no toolbar, from the same `title_bar.rs`.

---

## 9. Controls (see `controls.png`)

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
| Menus | `overlay`, radius 10, 4 inset, 28 rows, highlight = accent radius 6; shortcut column right-aligned in `ink3`, written as words in GNOME order ("Shift+Ctrl+N", "Alt+Super+H"; no ⌘, ⌃, ⌥ or ⇧ symbols); submenus open on the `quick` spring; a menu never leaves the screen: it flips, then slides, then shortens to scroll between 20 pt bands with a single `ink2` chevron (behaviour: `docs/architecture/behaviour.md`, Popup placement) |
| Alerts | card radius 14: the body on `overlay` with a 36 pt app disc, a Manrope 17 heading and body in `ink2`; a **footer tray** on `chassis` holding the buttons (default right, cancel left of it, an alternative such as "Don't Quit" as a quiet button on the far left). App-modal alerts are centred over the window with a scrim; document-modal alerts drop from under the header |
| Tooltip | `ink` fill with `chassis` text (inverted), radius 6, 12/500; no arrow |
| Notifications | `overlay`, radius 14, app disc 18 + LABEL-style app name + time, title 13.5/600, body 13 `ink2`; actions appear as quiet buttons on hover |
| Badges | danger capsule, 11/700 white, min 18 pt |

---

## 10. Iconography

> **Owner's amendment (2026-10-30):** app icons are **rounded squares**, as in
> `bar/01-desktop-left-collapsed.png`, not discs, and the bar's tool, status and tray icons follow
> that mockup's richer, more developed style rather than the minimal outline set below. The
> "Disc" and "Symbols" paragraphs are superseded where they conflict; the file and folder rules
> stay. Concretely: app icons are a rounded square on a 48 grid (44 pt body, circular-arc corners
> of radius 10, not a squircle), a rich but clean colour field (a subtle vertical gradient is
> allowed), and a detailed, well-crafted symbol with depth through tone (not gloss); status and
> tool icons (Wi-Fi, sound, Bluetooth, battery, tray, Trash, view modes) are drawn with the
> detail and weight of that mockup, filled where it helps legibility, and still recolour with the
> text colour where they are symbolic.

**App icons: "Disc."** On a 48 grid, a 44 pt circle of one **flat** colour field, with no gradient, gloss or squircle. It carries one geometric symbol in `paper` (`#FCFBF9`) or `ink`, built from 2.4–2.6 strokes with round caps, and **at most one ember signal square** (3–6 pt). A 1 pt inner edge at 10 % black (22 % white on dark fields) keeps discs defined on any background. The circle gives equal optical weight in a vertical bar and is the opposite of Apple's squircle. Third-party icons are shown unmodified, *centred in a 44 pt disc of `well`* only when they are full-bleed squares (as with Android's adaptive icons). The mockup shows Files (ultramarine, folder), Terminal (ink, chevron + underscore + ember cursor), Settings (stone, **three faders**, not a gear), Monitor (teal, pulse line + amber square), Text (paper, lines + ember caret) and Capture (plum, corner brackets + ember square).

**File icons: "Sheet + label."** A 30×40 white sheet, radius 3, with a hairline edge and **no dog-ear**, a content hint on top (text lines, picture, zipper) and a **coloured label band** at the bottom with the extension in JetBrains Mono 700 (PDF red, PNG moss, TXT warm grey, ZIP amber, audio plum, video rose, code teal, unknown `ink3`). Thumbnails replace the hint area for images and PDFs once thumbnailers exist (B2), and the label band stays.

**Folders**: a stone body (`#D9D3CA`) with an **accent tab**, so folders follow the user's accent while the folder itself stays neutral. Special folders have an engraved `ink2` symbol on the body (download arrow, picture, music, home).

**Symbols** (sidebar, menus, bar status): a 16/18 pt grid with 1.5 strokes, round caps and joins, outline only. Accent only when selected. They are drawn in-code (as `SYM` paths in the mockup) or as an SVG set in `data/icons/Bande/symbolic`. This replaces the Adwaita mix and follows roadmap C1.

**The theme** (`data/icons/Bande`, step D3) is generated by `python3 data/icons/src/generate.py`, which needs `fonts-jetbrains-mono` for the label bands (outlines are baked into the SVGs) and also writes our apps' hicolor icons. Symbolic icons are 16 pt, 1.6 strokes converted to single-colour fills (`#201E1A`, with `opacity` for unlit parts and `class="warning"`/`"error"` for low battery), so GTK, Qt and our toolkit can recolour them with the text colour. The accent tab of folders comes from hidden sibling themes `Bande-<Accent>` (`Inherits=Bande`) that hold only the recoloured folders: the session selects the icon theme `Bande-Teal` and so on for a non-default accent. `crates/shell/desktop/tests/icons.rs` renders every icon at 16–64 px through the toolkit (`icons-light.png`).

---

## 11. Motion signature

Keep `bande-spring` and the presets table in `docs/design/research/README.md` unchanged. Datum adds **how** things move, Material 3-style: *spatial* springs for position and size, *effects* springs (`fade`) for colour and opacity, never mixed.

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

The combined-bar mockups (`research/combined-bar/*.png`) put an edge-anchored bar of 64 pt on the left (expandable to 248 with labels) holding: the mark, the app menu block, apps, Trash, desktops, a status cluster and the clock. Datum fits it directly, and fixes the parts of those mockups that still read as macOS (traffic lights, squircle icons, blue folders, a gear):
- Bar surface = `chassis` (same tone as window frames, so the bar reads as the desktop's frame), with a 1 pt `edge` on the inner side only, square at the screen edge. It is opaque; "Tinted chassis" applies here too.
- **Active app = a 3×16 accent tick** on the screen-edge side; **running = a 3×3 `ink2` square** (the square motif; a dot would be macOS). The expanded bar's app rows use the sidebar row spec (28/32 pt, `well` selection radius 6).
- The app-menu block and the status cluster are `well` groups with radius 8 (concentric with 14 panels).
- The collapsed clock is Inter 15/600 tnum stacked "14 / 32"; the expanded one is Manrope 22/700 with the date in Callout.
- The mark is a "U" of seven 4.5 pt squares in `ink` (mockups); the signal-blue dot in the researcher's "U" mark becomes the ember square if a colour is wanted.
- Bar pop-overs (Control Center, calendar, notifications) are `overlay` panels with radius 14 that slide out from the bar along its normal axis (`standard`).
- Horizontal placement uses the same rules with the tick on the bottom edge.

`window-light.png` and `window-dark.png` show a collapsed bar in this language next to a Files window.

---

## 14. Mockups

All in `datum/`:

| File | Shows |
|---|---|
| `window-light.svg/.png`, `window-dark.svg/.png` | 1280×800 desktop: collapsed combined bar, Files window with frame + sheet, left window controls, flush-left Manrope title, sidebar symbols, accent selection (inverted text in dark), tracked column labels |
| `controls.svg/.png` | Window controls (rest / hover / inactive), buttons, fields, switch, checkbox, square-centred radio, segmented, pop-up, stepper, fader sliders, segmented progress, tooltip, badge, status squares, selection states; light and dark |
| `overlays-light.svg/.png`, `overlays-dark.svg/.png` | Context menu with Super-glyph shortcuts and a submenu, an alert with a footer tray, a notification |
| `icons.svg/.png` | Six Disc app icons and six Sheet+label file and folder icons, at 72 and 20 pt, on light and dark |
| `sheet.svg/.png` | Colour and type sheet: neutrals, accents, semantics, type scale, radii, concentric rule, spacing, elevation, motifs |
| `src/*.py` | Generators (`colors.py` holds the OKLCH → hex and contrast maths) |

---

## 15. Migration plan

### Step 0: the design-system home
Create **`crates/toolkit/design`** (crate `bande-design`, no dependencies beyond `bande-graphics`'s `Color`), or grow `graphics/src/tokens.rs` into a `tokens/` module if a new crate feels premature. It contains:
- `palette.rs`: `Palette` with the full role set (`chassis, sheet, overlay, well, hairline, edge, ink, ink2, ink3, accent, accent_text, on_accent, accent_soft, focus, danger, warning, success, scrim`), generated by `Palette::generate(scheme, accent, contrast)` from OKLCH (port `oklch()` from `design/src/colors.py`); `Accent` enum with the 8 presets.
- `radius.rs`: `Radius::{XS,S,M,L,XL}` and `fn concentric(outer, inset) -> f32`.
- `spacing.rs`: the scale, `ControlSize::{Small, Regular, Large}` heights and row heights.
- `type_scale.rs`: `TextRole::{Display, Title, Heading, WindowTitle, Body, BodyStrong, Callout, Label, Mono}` → family, size, line height, weight, tracking, features.
- `elevation.rs`: `Elevation::{Frame, Overlay, Modal}` → shadow pair per scheme; scrim.
- `contrast.rs` + tests: WCAG and APCA checks over every generated palette and accent (this is a real domain test that fails when a token change breaks legibility).
- `docs/design/language.md` in the repo: this document, trimmed to the rules, with the mockups.

### Order of work (each step ships with updated goldens, light and dark, 1× and 1.5×)
1. **Tokens.** Expand `Palette` and add `Radius`, `TextRole` and `Elevation`. Keep the old field names as the first users migrate, then delete them in the same branch (no dead code).
2. **Fonts.** Add `fonts-manrope` and `fonts-jetbrains-mono` to `bande-desktop`'s Depends. In `graphics/src/text.rs`: `DISPLAY_FONT_FAMILY = "Manrope"` for TextRole ≥ 15 pt display roles, `MONOSPACE_FONT_FAMILY = "JetBrains Mono"`, and default features `cv05, cv08, ss07, ss08` plus `tnum` on `TextStyle::numeric()`.
3. **Title bar and window controls.** `graphics/src/title_bar.rs`: height 48 (40 SSD), the control well and glyphs, flush-left title, frame colour; `compositor/src/scene.rs` `CORNER_RADIUS` 14; `compositor/src/frame.rs` and `decoration.rs` use the same items for SSD; `toolkit/platform/src/chrome.rs` gains the frame/sheet split (an app declares a sidebar width; the toolkit paints the sheet).
4. **Controls.** `widgets/src/{button,text_field,toggle,checkbox,radio_group,segmented,pop_up_button,stepper,slider,progress,tooltip,menu,sheet,table,scroll}.rs`: replace local `RADIUS` constants and `hex(0x…)` with tokens; implement the fader knob, segmented progress, square radio, Super glyph in `menu.rs`, gliding selection.
5. **Alerts and sheets.** `widgets/src/sheet.rs` + `toolkit/platform` alert: the footer tray and scrim; fix the "Documents" title leak in the alert golden.
6. **Shell.** `shell/dock` (or its successor, the combined bar): remove translucency and the 24 hard-coded colours, add the tick and square indicators; `shell/menubar` retires into the bar; `shell/notifications/src/card.rs` radius 14 → tokens; `shell/launcher` radius 14, scrim + blur, result rows on the list spec; `controlcenter`, `lock`, `greeter`, `screenshot/area.rs` to tokens.
7. **Apps.** Files (frame + sheet, symbols, Sheet+label icons, accent-tab folders), Settings (sidebar on chassis, Manrope page titles, rows 44 on `sheet` without card outlines), Monitor (segmented control and tables to tokens, chart in accent + danger), Terminal (`theme.rs`: JetBrains Mono; the default profile uses `sheet` and `ink` as background and foreground, with ANSI colours derived from the accent set).
8. **Icons (roadmap C1).** Produce `data/icons/Bande/scalable/apps/*` (Disc), `mimetypes/*` (Sheet+label), `places/*` (stone + accent tab, recoloured at runtime from the accent, or as 8 pre-rendered variants), `symbolic/*` (the 16 pt line set). Remove the Adwaita fallbacks from the shipped theme's `Inherits` only once coverage is complete.
9. **Third-party apps.** Push the palette to GTK (`gtk.css` accent and named colours) and Qt (the kdeglobals colour scheme) through the portal and settings daemon (roadmap A1), so foreign apps pick up warm neutrals and the accent.
10. **Settings ▸ Appearance.** Light/Dark/Auto, 8 accent swatches, "Tinted chassis", Increase Contrast, and the Larger/Hyperlegible text option. No other knobs (DESIGN §1).

### Risks and open questions for the owner
- **Circle app icons** are the boldest call. The alternative, a 14 pt-radius *arc* square, is less distinctive but closer to most third-party icons.
- **Warm neutrals**: if the hue reads as beige on some panels, drop the chroma to 0.003; the language survives.
- **The 48 pt header** costs 20 pt of height versus today. It is justified by merging the toolbar into it; utility windows use 36.
- **Manrope** has no italic. Display roles never need one; body italics stay in Inter.
