# Dark theme: lift it off black and tone down the glare

## Summary
Dark Datum is near-black, warm brown-grey, with almost-white text (WCAG 15:1, APCA Lc −97). That
is darker and harsher than every current desktop dark theme. Move it to a lifted, cool "dusk
slate" neutral (frame `#1F2329`, content `#272A30`), dim the text to Lc −88, make menus and
popovers lighter than the window, and replace the bright pastel selection fill with a mid-tone
one carrying white text. All of it is token changes in `crates/toolkit/design`, reviewed by the
owner on side-by-side goldens and a live branch build before it merges.

## What the owner sees
- Window frames, sidebars and the bar are `#141210` (OKLCH L 0.185): next to black. The
  content sheet is `#1D1B19`. Both are brown-tinted (hue 80), which reads as old "warm
  charcoal" next to today's blue-grey darks
  (`crates/apps/files/tests/golden/columns-dark.png`,
  `crates/apps/settings/tests/golden/appearance-dark.png`).
- The text is `#F1F0ED` on that near-black. It sits above APCA's advisory ceiling for dark
  mode, which is where halation comes from.
- Selection is a large bright pastel block (`#7E96FB`, luminance 0.33) with dark text. In a
  dark UI it is the brightest thing on screen (the `a.txt` row in `columns-dark.png`, the lock
  screen avatar).
- Menus, notifications and the launcher card are only 1 tone step above the window, and they
  are told apart by a hairline (`#32302D`) that barely shows.

## Findings

### Current tokens
- `crates/toolkit/design/src/palette.rs:3`: `NEUTRAL_HUE = 80.0` (warm) for both schemes.
- `palette.rs:161-176`, dark neutrals (OKLCH L, C): chassis .185/.006, sheet .225, overlay .265,
  well .275, hairline .31, edge .41 (.52 Increased), ink .955/.004, ink2 .78, ink3 .64.
  Scrim black at 40 %, `accent_soft` mix 0.78.
- `palette.rs:203-210`: in dark, `field = well` and `raised = edge`.
- `palette.rs:79-86`: accents have one dark tone each (Ultramarine `#7E96FB`, L .70). That
  same tone is used as fill, text and focus (`palette.rs:216-220`). `on_accent` is white or
  the dark text, whichever contrasts more, so every dark selection carries dark text. This is
  asserted by `tests/palette.rs:166` `dark_selections_carry_dark_text`.
- `palette.rs:61-63`: danger, warning and success have a single dark tone (`#F66D67`, …).
- `crates/toolkit/design/src/elevation.rs:318`: dark Frame and Overlay share one heavy shadow
  pair (key α .50, ambient 14/18 α .55).
- The dark-only 1 px `hairline` stroke is copied into five places:
  `toolkit/platform/src/chrome.rs:353`, `platform/compositor/src/label.rs:82`,
  `shell/notifications/src/card.rs:469`, `shell/launcher/src/view.rs:509`,
  `shell/controlcenter/src/lib.rs:410`.
- `shell/bar/src/dock.rs:1841,2183` hard-code dark shadow alphas (110 and 100).
- `crates/toolkit/design/tests/palette.rs:21-67` pins every hex, and `:70-87` requires ink
  Lc ≥ 90 on chassis, sheet and overlay. That rule forces near-white text on near-black.
- Docs: `docs/design/datum.md` §5 (neutrals table, measured contrast, the "dark mode inverts
  on-accent text" trait, which it calls one of Datum's most recognisable) and §6 (elevation);
  `docs/design/DESIGN.md` §4. Mockup generator: `docs/design/datum/src/colors.py`.
- Propagation already follows the palette:
  - GTK 4/libadwaita: `services/portal/src/gtk.rs` writes `--window-bg-color` and the other
    variables from `Palette::generate`.
  - Terminal: `apps/terminal/src/theme.rs` builds its themes from the palette.
  - Compositor overlays (SSD frames, switcher, window menu, labels): they use
    `decorations.scheme`, set from `frame.rs:390`.
  - Lock and greeter: forced dark at `shell/lock/src/lib.rs:79`.
  - GTK 3, Qt and Electron apps get only the `color-scheme` and `accent-color` portal keys
    (`portal/src/settings.rs:286-315`).
- Hard-coded colours outside the tokens are almost all gone: only shadow and scrim blacks, the
  emoji skin tones and screenshot markup colours remain.

### How 2025-2026 dark themes are built (measured with Datum's own OKLCH/WCAG/APCA code)
| System | Base / window | Content | Raised (header, popover) | Text | Text on base |
|---|---|---|---|---|---|
| libadwaita 1.6+ | `#222226` L .254, hue 286 | view `#1D1D20` | headerbar `#2E2E32`, popover `#36363A` | `#FFFFFF`, dim 55 % | 15.9:1, Lc −105 |
| Windows 11 Fluent | `#202020` L .244 | layers white 5 % | tertiary `#282828`, `#2C2C2C` | `#FFFFFF`, secondary 77 %, tertiary 53 % | 16.3, −106 |
| KDE Breeze Dark | window `#202326` L .254, hue 248 | view `#141618` | header `#292C30` | `#FCFCFC` | 17.7, −105 |
| GitHub dark dimmed | `#212830` L .274, hue 252, C .018 | inset `#151B23` | muted `#262C36` | `#D1D7E0` | 10.3, −79 |
| VS Code Dark Modern | editor `#1F1F1F` | side `#181818` | input `#313131` | `#CCCCCC` | 10.3, −74 |
| Radix Slate (dark) | 1–2 `#111113`/`#18191B` | 3 `#212225` | 4–5 `#272A2D`/`#2E3135` | 12 `#EDEEF0` | |
| Material 3 | tone-based: containers step up from tone 4 to 22 | | elevated = lighter | on-surface tone 90 | |
| macOS (Sequoia/Tahoe) | window roughly 40–54 grey (measured, not documented) | content darker | lighter popovers | label = white at about 85 % | |
| **Bande now** | **`#141210` L .184, hue 67** | `#1D1B19` | `#272522` | `#F1F0ED` | **16.4, −97** |

What these share, and what Bande lacks:
1. **No near-black for the base.** The base sits at L .24-.27. Bande's .185 is darker than
   all of them except GitHub's high-contrast default and Linear.
2. **Elevated means lighter.** Popovers, headers and cards step up to L .29-.33 (Material 3
   tone surfaces, Fluent layers, libadwaita popovers).
3. **Tinted cool neutrals are the norm.** Hue 248-286 at chroma .005-.018 (Breeze, GitHub,
   libadwaita since 1.6, Radix Slate). Warm hue 80 is the outlier.
4. **The comfortable themes dim the text.** GitHub dimmed, VS Code and macOS run body text at
   Lc −74 to −85. APCA's maintainers give **Lc 90 as an advisory maximum in dark mode**
   (85 for large, bold text) and suggest text no brighter than about `#E6E6E6`, because
   halation, not lack of contrast, is the dark-mode problem. The body-text floor stays Lc 75
   (90 preferred in light).
5. **The surface steps are small.** Window-to-content and window-to-popover differ by
   1.1-1.3:1. Separation comes from tone, plus a faint light rim on floating surfaces
   (Fluent's card and control strokes are white at 5-8 %).
6. **Selection uses a mid-tone accent with white text** (libadwaita `#3584E4`, Fluent, macOS).
   No major dark theme fills selections with a pastel.

Sources: [libadwaita CSS variables](https://gnome.pages.gitlab.gnome.org/libadwaita/doc/main/css-variables.html);
WinUI `Common_themeresources_any.xaml` (microsoft/microsoft-ui-xaml, Dark dictionary);
[BreezeDark.colors](https://invent.kde.org/plasma/breeze/-/raw/master/colors/BreezeDark.colors);
Primer primitives 10.3 `dark.css` and `dark-dimmed.css`; VS Code `dark_modern.json`;
[Radix Colors 3.0](https://www.radix-ui.com/colors) `slate-dark.css`;
[Material tone-based surfaces](https://material.io/blog/tone-based-surface-color-m3);
[APCA introduction](https://github.com/Myndex/apca-introduction),
[SAPC-APCA discussion #106](https://github.com/Myndex/SAPC-APCA/discussions/106),
[W3C css-color list on the dark-mode maximum](https://lists.w3.org/Archives/Public/public-css-archive/2022Jun/0529.html);
[Apple forum measurement of dark title bars](https://developer.apple.com/forums/thread/674908).

## Options

**A. Brighten only.** Raise the warm neutrals by about 0.07 L and dim the ink.
Smallest diff, same identity. But it keeps the brown cast the owner reads as dated, and it
still has the pastel selection glare.

**B. "Dusk slate" neutrals + dimmed text + lighter overlays (recommended core).**
Cool hue 262, chroma .011-.013 on surfaces and .006 on text; base L .255; text at Lc −88.
This is the 2026 mainstream shape, made Bande's own in three ways:
- The hue sits between Breeze/GitHub (248-258) and GNOME (286), next to the Ultramarine
  accent (272) and the blue-violet wallpapers.
- The tint is visible but subtle: more chroma than GNOME, less than GitHub.
- It keeps Datum's frame/sheet structure.

It costs about 37 dark goldens and some doc and test changes.

**C. B + mid-tone selection (recommended).** Each accent gets a dark *fill* tone, with
luminance solved to about 0.175 so white text reaches ≥ 4.5:1 and the fill stays ≥ 3:1 on
chassis and sheet. The existing light tone becomes `accent_text`/`focus` only (links, rings,
chart lines, ANSI colours). This halves the brightest area on a dark screen, at the cost of
dropping Datum's "inverted selection" trait. Amber and Graphite keep a light fill with dark
text, since white cannot reach 4.5:1 on them.

**D. Also move light to the same slate hue.** One neutral family in both schemes, so Auto
switching looks like one product. The light contrast numbers are unchanged (table below), and
the move answers "the themes look dated" for light too. Cost: all ~117 light goldens. Code
cost is trivial.

Rejected: a separate "dimmed" scheme or a user dark-level slider. That is a setting standing in
for a design decision (DESIGN.md §1), and it doubles the goldens.

## Recommendation: B + C now, D as a follow-up the owner can veto after seeing B + C

### Proposed dark neutrals (hue 262)
| Role | OKLCH (L C) | Hex | Was |
|---|---|---|---|
| `chassis` (frame, sidebar, bar) | .255 .013 | `#1F2329` | `#141210` |
| `sheet` (content) | .285 .012 | `#272A30` | `#1D1B19` |
| `well` / `field` (controls, search) | .325 .012 | `#31343A` | `#292724` |
| `overlay` (menus, popovers, notifications, alerts) | .335 .011 | `#34373C` | `#272522` |
| `hairline` (separators) | .365 .012 | `#3B3F45` | `#32302D` |
| `rim` (new: 1 px edge of floating dark surfaces) | .42 .012 | `#494D54` | (hairline) |
| `edge` / `raised` (control outlines, segmented chip) | .47 .012 (Increased .58) | `#575B62` (`#767A82`) | `#4C4A47` |
| `ink` | .925 .006 (Increased .965) | `#E4E6EA` (`#F2F3F6`) | `#F1F0ED` |
| `ink2` | .80 .012 (Increased .86) | `#B9BEC6` (`#CDD1D8`) | `#BAB7B2` |
| `ink3` | .655 .012 (Increased .74) | `#8D9198` (`#A7ABB1`) | `#8F8C87` |
| `deep` (new: dark text on light fills, replaces "use chassis") | .215 .015 | `#161A21` | `#141210` |

The scrim stays black at 40 %. `accent_soft` stays a 0.78 mix toward the sheet.

### Contrast (computed with the repo's own `wcag()`/`apca()` maths, ported in `colors.py`)
WCAG 2 is the ratio of relative luminances (Y_hi + 0.05)/(Y_lo + 0.05). APCA is SAPC-0.0.98G
(soft-clip 0.022, exponents 0.56/0.57 for dark text on light and 0.65/0.62 for light text on
dark, scale 1.14, offset 0.027).
Example: chassis `#1F2329` has Y = 0.0165 and ink `#E4E6EA` has Y ≈ 0.79, so the WCAG ratio is
(0.79 + 0.05)/(0.0165 + 0.05) = 12.6.

| Pair | Proposed | Now |
|---|---|---|
| ink on sheet | 11.5:1, Lc −88 | 15.1:1, Lc −97 |
| ink on chassis | 12.6, −89 | 16.4, −97 |
| ink on overlay | 9.6, −85 | |
| ink2 on sheet / chassis / overlay | 7.7 −63 / 8.4 −65 / 6.4 −60 | 8.6 −62 on sheet |
| ink3 on sheet / overlay | 4.5 −39 / 3.8 −36 | 5.1 −39 on sheet |
| edge vs sheet (Standard / Increased) | 2.11 / 3.34 | |
| hairline vs sheet | 1.36 | |
| rim vs overlay / vs chassis | 1.41 / 1.86 | |
| sheet vs chassis, overlay vs sheet | 1.10, 1.20 | |
| Increased ink / ink2 / ink3 on sheet | 13.0 −96 / 9.4 −75 / 6.3 −53 | |

Every text pair stays within the current floors (ink ≥ 7:1, ink2 ≥ 4.5:1 and Lc 60, ink3
≥ 3:1 and Lc 30). Ink now sits under the dark-mode ceiling (Lc ≤ 90) instead of above it.

### Proposed dark accent fills (option C), white text unless noted
| Accent | Fill | White on fill | Fill vs sheet / chassis | Text tone (unchanged) |
|---|---|---|---|---|
| Ultramarine | `#576CD3` (L .568 C .16) | 4.68:1, Lc 78 | 3.08 / 3.37 | `#7E96FB` |
| Teal | `#008182` (.545 .10) | 4.71, 78 | 3.06 / 3.35 | `#3FC0C0` |
| Moss | `#48813B` (.548 .12) | 4.69, 78 | 3.08 / 3.37 | `#80C271` |
| Ember | `#C94A1F` (.577 .17) | 4.68, 77 | 3.08 / 3.37 | `#F77C56` |
| Rose | `#C8436D` (.580 .17) | 4.69, 77 | 3.08 / 3.37 | `#F47C9B` |
| Plum | `#9F58B0` (.577 .15) | 4.67, 78 | 3.09 / 3.38 | `#C889D7` |
| Amber | `#E7963D` (.74 .14), `deep` text | 7.35, Lc 56 | 6.06 / 6.63 | `#F7AB5D` |
| Graphite | `#CDD1D8` (.86 .01), `deep` text | 11.4, Lc 77 | 9.4 / 10.3 | `#E1DDD7` (moves to hue 262) |
| Danger | `#CF4140` (.58 .18) | 4.68, 77 | 3.08 / 3.37 | `#F66D67` |

Fill luminance drops from 0.33 to 0.175, so the selection's glare is halved. The text tones
stay ≥ 4.3:1 on the new overlay (Ultramarine 4.35, danger 4.15), so links, focus rings and
chart lines keep the 3:1 non-text floor everywhere. Fills on `overlay` (menu highlight) are
2.56:1. That is fine, because the highlight carries Lc 78 white text, but the test must stop
requiring 3:1 for fills on overlay.

### Elevation (dark)
- Frame: key 1/1 α .40, ambient 14/18 α .45. Overlay gets its own lighter pair: 1/0.75 α .35
  and 6/9 α .40. Inactive windows still halve the ambient.
- A `rim` stroke on floating dark surfaces (sheet edge, menus, notifications, launcher card,
  control centre, compositor labels) replaces today's five `hairline` copies.

### Light (option D, follow-up)
| Role | Hex now (hue 80) | Hex at hue 262 |
|---|---|---|
| chassis | `#ECEAE7` | `#E9EBEF` |
| sheet | `#FCFBF9` | `#FAFBFC` |
| overlay | `#FFFFFF` | `#FFFFFF` |
| well | `#E2DFDB` | `#DEE1E7` |
| hairline | `#D4D0CB` | `#D0D3D8` |
| edge | `#BAB7B2` | `#B4B7BE` |
| ink | `#201E1A` | `#1C1F25` |
| ink2 | `#5B5752` | `#555A62` |
| ink3 | `#83807A` | `#7C8089` |

Contrast is unchanged: ink on sheet 15.9:1 and Lc 101, ink2 on chassis 5.8 and Lc 72, ink3 on
chassis 3.3 and Lc 55, edge vs sheet 1.94. The light accents are untouched.

## Implementation plan
Milestones 1-4 go on one branch, `day/dark-slate`. Each is its own commit, and the full gate
runs after each one. The branch merges only after the owner approves the preview (milestone 5).
All builds and tests run on the mini (`BANDE_UPDATE_GOLDENS=1` only for the named crates).
The sidebar-selection plan (`NOTES/plans/sidebar-selection.md`) takes its dark values from
here. Land it before or after this branch, never interleaved, so its goldens are regenerated
once.

1. **Dark slate neutrals and the glare ceiling** (about 2 h)
   - `crates/toolkit/design/src/palette.rs`:
     - Give each scheme its own neutral hue (dark 262; light stays 80 until milestone 6).
     - Set the dark neutral table to the values above and add the `rim` and `deep` roles.
     - In dark, `raised` becomes `edge` (unchanged mapping) and `legible_on` uses `deep`
       instead of chassis.
   - `crates/toolkit/design/tests/palette.rs`:
     - Update the pinned dark hexes.
     - Split the ink rule: light keeps ink ≥ 7:1 and Lc ≥ 90. Dark Standard requires
       ink ≥ 7:1 and Lc 75 ≤ |Lc| ≤ 90 on chassis, sheet and overlay (the new ceiling, which
       fails if someone brightens ink back to near-white).
     - Add `dark_surfaces_step_up_in_lightness`: luminance of chassis < sheet < well ≤
       overlay < hairline < rim < edge, in Standard and Increased, for every accent. This
       guards the "elevated is lighter" order.
     - Edit `dark_selections_carry_dark_text` to assert `on_accent == deep` for now.
   - `docs/design/datum.md` §5 tables and contrast targets; `docs/design/datum/src/colors.py`
     values.
   - Regenerate the dark goldens:
     - `apps/files`: 12 `*-dark`.
     - `apps/monitor`: 11 `*-dark`.
     - `apps/settings`: `about-dark`, `appearance-dark`, `displays-dark`.
     - `apps/terminal`: `term-window-dark`.
     - `shell/bar`: `bar-dark`, `low-memory-confirm-dark`, `shut-down-blocked-dark`.
     - `shell/desktop`: `desktop-dark`.
     - `toolkit/platform`: `window-frame-rest-dark-1x`.
     - `toolkit/widgets`: `controls-dark-1x`.
     - `shell/lock` (forced dark): `lock-screen`, `lock-screen-status`, `login-power`,
       `login-power-shut-down`.
   - Proof: palette tests, workspace tests with the regenerated goldens, and a Read of each
     regenerated dark golden to check it.

2. **Mid-tone accent fills in dark** (about 2 h)
   - `palette.rs`:
     - `Hued` gains a `dark_fill` tone, using the table above. `Accent::color()` keeps
       returning the text tone, because terminal ANSI, Settings swatches, the portal and
       `nearest()` use it. Add `Accent::fill(scheme)`.
     - In dark: `Palette.accent` = fill, `accent_text`/`focus` = the text tone,
       `on_accent` = `legible_on(fill)`. Danger gets the same split
       (`danger` = fill, new `danger_text` only if a text user exists; check first).
   - Audit the 77 `.accent` users (`grep -rnE '\.accent\b[^_]' crates`). Text, glyphs and
     thin lines on a surface move to `accent_text`. Known cases:
     - `apps/monitor/src/monitor_view.rs` chart lines.
     - The `shell/bar/src/rail.rs` active tick: keep the fill, since it is ≥ 3:1.
     - Link-like text in `apps/settings/src/widgets.rs` and `services/polkit/src/dialog.rs`.
     - `apps/files/src/{grid,drag,paint}.rs`: drop targets stay on the fill.
   - Tests:
     - Replace `dark_selections_carry_dark_text` with
       `dark_fills_are_dimmer_than_their_text_tone_and_carry_legible_text`: fill
       luminance ≤ 0.2 except Amber/Graphite, `on_accent` ≥ 4.5:1.
     - `the_signal_reads_on_every_surface…` now checks `accent_text`/`focus`/`danger`
       ≥ 3:1 on chassis, sheet and overlay, and the fill ≥ 3:1 on chassis and sheet.
     - Update the pinned accent hexes.
   - Regenerate the dark goldens that show selection or accent: Files, Monitor, Settings,
     widgets, lock, bar.

3. **Dark elevation and one rim** (about 1.5 h)
   - `crates/toolkit/design/src/elevation.rs`: separate dark Frame and Overlay pairs (values
     above).
   - Add one helper next to `Shape`/`Item` in `crates/toolkit/graphics` (search for an existing
     stroke helper first) that returns the dark `rim` stroke, or none in light. Replace the five
     copies:
     - `toolkit/platform/src/chrome.rs:353`
     - `platform/compositor/src/label.rs:82`
     - `shell/notifications/src/card.rs:469`
     - `shell/launcher/src/view.rs:509`
     - `shell/controlcenter/src/lib.rs:410`
   - Move `shell/bar/src/dock.rs:1841,2183`'s hard-coded dark alphas onto `Elevation`.
   - Proof: the goldens below, plus a unit test in graphics. The helper must return no item in
     light and a 1 px rim inset by 0.5 in dark, a real geometry rule the five call sites rely
     on.
   - Regenerate:
     - Compositor dark goldens, if any exist after milestone 1.
     - The light goldens of notifications, launcher and control centre only if they changed
       (they should not).
     - The dark goldens from milestone 1 that show menus or cards.

4. **Terminal and third-party propagation** (about 1.5 h)
   - `apps/terminal/src/theme.rs`:
     - Check the ANSI black (`edge`) and bright black against the new sheet.
     - `Midnight` keeps a deliberately deep background by using `deep` instead of `chassis`
       (owner decision 4).
     - `on_current_match` uses `deep`.
   - `services/portal/src/gtk.rs`:
     - `card-bg-color` moves to `well`, so libadwaita cards step up as on the desktop.
     - `headerbar-*` stays chassis.
     - Add `--accent-color` (the text tone) next to `accent-bg-color` (the fill).
   - Write a GTK 3 `~/.config/gtk-3.0/bande.css` with `@define-color theme_bg_color` /
     `theme_base_color` / `theme_fg_color` / `theme_selected_bg_color` / `borders`, imported
     the same way as the GTK 4 file (reuse `with_import` and `replace`). GTK 3 apps then stop
     showing Adwaita-dark's `#353535`.
   - Qt and Electron (VS Code, Claude desktop) keep following `color-scheme` and
     `accent-color`. Their frames are covered by the third-party title bar plan in
     `NOTES/plans/` (gaps.md, "GTK apps do not fit with our window bar").
   - Proof:
     - `crates/services/portal/tests/settings.rs`, extended so the generated GTK 3 and GTK 4
       sheets carry the palette's chassis, sheet and fill hexes for both schemes and keep a
       user's own `gtk.css` lines.
     - `term-window-dark` golden.
     - `crates/apps/terminal` theme tests.

5. **Owner preview, then merge** (about 1 h of agent work)
   - On the mini, build the branch and copy each changed golden pair (main: `git show
     main:<path>`; branch: the file) into the scratchpad.
   - Publish one private before/after page (Artifact) to the owner. It holds:
     - Files columns.
     - Settings appearance.
     - Controls.
     - Window frame.
     - Bar.
     - Lock.
     - Terminal.
     - Monitor CPU.
   - Also install the branch build into the live session on the mini for an evening (warn the
     owner before restarting the session; memory "Warn before visible disruption").
   - Merge only on the owner's yes. Any tweak (hue, base L, ink L) is a one-line token change
     plus a dark golden refresh.

6. **Light to the slate family** (about 2 h, only if the owner says yes after the preview)
   - `palette.rs`: one shared neutral hue 262, the light table above. Graphite light moves to
     hue 262.
   - `tests/palette.rs`: pinned light hexes.
   - `datum.md`: delete the "warm hue is a deliberate break" paragraph.
   - Regenerate all non-dark goldens (about 117) across the 16 golden directories:
     - `apps/{file-chooser,files,monitor,settings,terminal}`
     - `platform/compositor`
     - `services/polkit`
     - `shell/{bar,controlcenter,desktop,launcher,lock,notifications,screenshot}`
     - `toolkit/{platform,widgets}`
   - Then run the same preview step as milestone 5.

7. **Docs and mockups** (about 1 h)
   - `docs/design/datum.md` §5/§6: tables, the rewritten selection trait, the dark-mode
     ceiling as a stated target.
   - `docs/design/DESIGN.md` §4: "Depth through tone": elevated is lighter in dark, plus one
     rim.
   - Regenerate `docs/design/datum/*-dark.svg` and `sheet.svg` from `src/*.py`. Produce the
     PNGs only if `rsvg-convert` is present on the mini; otherwise keep the SVGs and note it.
   - No tests (docs only).

## Risks
- **Golden churn.** About 37 dark goldens are touched in milestones 1-3, and about 117 light
  ones if milestone 6 lands. To keep review possible:
  - Regenerate only the named crates.
  - Check every regenerated PNG by eye (Read) before committing.
  - Never regenerate a golden that failed for another reason.
- **Mid-tone fills on overlay (2.56:1).** The menu highlight relies on its text. If the owner
  wants ≥ 3:1 there too, raise overlay-highlight fills about 0.03 L, which costs white-text
  margin (4.5 → about 4.1). The alternative is keeping the light fill with `deep` text in
  menus only.
- **Small surface steps (1.1-1.2:1)** may read as flat on the mini's display. The preview
  decides. Raising `sheet` to .295 is the cheap fix.
- **`Accent::color` meaning changes.** It now means the text tone in dark. Callers that paint
  fills with it (Settings swatches) must be checked in milestone 2.
- **Frame tint.** "Tint window frames with the wallpaper" mixes 6 % into the lighter chassis.
  `a_wallpaper_tint_shifts_only_the_frame…` keeps guarding legibility.
- **The other plans.** Sidebar-selection and third-party title bars both read these tokens.
  Agree the order with them so goldens are regenerated once.

## Decisions for the owner
1. Neutral hue: dusk slate 262 (recommended), or a more neutral 255, or keep warm 80 and only
   brighten (option A).
2. Selection in dark: mid-tone fill with white text (recommended), or keep Datum's light
   pastel fill with dark text.
3. Light scheme: move to the same slate family after seeing the dark preview (option D), or
   keep the warm light.
4. Terminal "Midnight" profile: keep a deliberately deep background (`#161A21`), or let it
   follow the new chassis.
5. Preview format: the before/after page alone, or also an evening on the live branch build.
