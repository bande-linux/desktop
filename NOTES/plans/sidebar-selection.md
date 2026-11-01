# Sidebar selection: one quiet highlight

## Summary
Settings marks its selected sidebar row with two blue markers on top of the grey fill: an
accent-tinted symbol and, after any click, the accent focus ring. Drop both. Every sidebar
(Settings, Files, file chooser) gets the same neutral, tone-only highlight with an active,
focused and inactive-window state. Colours come from the palette, so the dark-theme plan
(NOTES/plans/dark-theme.md) sets the dark values.

## What the owner sees
- Settings: the selected row is a `well` fill with a blue (accent) symbol. When you click a
  row or step with Up/Down, a 2 pt blue ring is drawn around it too (golden
  `crates/apps/settings/tests/golden/about-light.png` shows the blue symbol; goldens are taken
  without focus, so the ring is not in them).
- Files: same grey fill with a blue symbol ("home" in `crates/apps/files/tests/golden/icons.png`).
- File chooser: grey fill, but once the sidebar list has focus the row turns into a solid
  accent fill with white text.
- Activity Monitor has no sidebar (`crates/apps/monitor/src/main.rs:282`, `sidebar: 0.0`).
- None of them changes when the window loses focus.

## Findings
- `crates/apps/settings/src/widgets.rs:742-752`: `SidebarRow::paint` fills `palette.well`
  when selected and calls `focus_ring` when focused.
- `crates/apps/settings/src/widgets.rs:801`: `SidebarRow::accepts_focus` is always true, and
  `crates/toolkit/widgets/src/lib.rs:739-741` focuses any focusable widget on primary press,
  so every click on a row draws the ring. `step_pane` (`crates/apps/settings/src/view.rs:1254`)
  also moves focus to the row.
- `crates/toolkit/widgets/src/control.rs:13-28`: the ring is `palette.focus`, which is the
  accent (`crates/toolkit/design/src/palette.rs:220`).
- `crates/apps/settings/src/header.rs:63-67`: the selected symbol is tinted `palette.accent`,
  others `ink2`. The attention row (`header.rs:87-88`) tints its symbol accent as a status cue.
- `crates/apps/files/src/paint.rs:224-236`: Files paints `well` (hover `well` at 50 %) and an
  accent symbol for the selected place. No ring; the sidebar never takes keyboard focus.
- `crates/apps/file-chooser/src/view.rs:533-561`: the chooser sidebar is a `Table`;
  `crates/toolkit/widgets/src/table.rs:1418-1421` uses `accent` when the table is focused,
  `well` otherwise, and `table.rs:1232` switches text to `on_accent`.
- No widget knows whether its window is active: `Context` (`crates/toolkit/widgets/src/lib.rs:144-160`)
  has no such field. The platform sends `WindowEvent::Focus(bool)` on keyboard enter/leave
  (`crates/toolkit/platform/src/seat.rs:545,552`); Settings and Files only react to
  `Focus(true)` for the app menu (`settings/src/main.rs:159`, `files/src/main.rs:258`).
- Design docs: `docs/design/datum.md:223` currently specifies "the selected symbol turns
  accent"; `datum.md:41` already calls mixed selection signals a problem; `datum.md:176` says
  selection must recede in inactive windows; `DESIGN.md` 4 says the selection should be the
  strongest element in the active window.

## Recommendation
One neutral "where I am" highlight for all sidebars, tone only, no accent anywhere:

| State | Fill | Symbol / text |
|---|---|---|
| selected, window active | `well` | `ink` / `ink` |
| selected, sidebar has keyboard focus | `hairline` (one tone step darker) | `ink` / `ink` |
| selected, window inactive | `well` mixed 50 % toward `chassis` | `ink2` / `ink` |
| not selected | none | `ink2` / `ink` |

Keyboard focus is shown by the darker tone step, never a ring, so it reads as "this list
has the keys" and not as a marker. Content selection on the sheet stays accent (Datum's
"where I am is grey, what I picked is signal"). The rule lives in one place:
`Palette::sidebar_selection(window_active, focused) -> Color`, used by all three sidebars.

## Implementation plan
1. **Add the sidebar selection role and window-active state.** Done.
   `crates/toolkit/design/src/palette.rs`: `Palette::sidebar_selection(window_active: bool,
   focused: bool) -> Color` (table above). `crates/toolkit/widgets/src/lib.rs`: `pub
   window_active: bool` on `Context`, default `true`. Test in
   `crates/toolkit/design/tests/palette.rs`: in every palette (`every_palette`) the three
   selection tones are distinct from each other and from `chassis`, and `ink` stays legible
   on each (reuse the existing contrast helpers). Update `docs/design/datum.md:121,223,227`:
   sidebar selection is tone-only, no accent symbol, no ring on sidebar rows.
2. **Settings: drop the markers.** Done. `crates/apps/settings/src/widgets.rs` `SidebarRow::paint`:
   fill from `sidebar_selection(cx.window_active, cx.is_focused(self.id))`, remove the
   `focus_ring` call. `crates/apps/settings/src/header.rs` `sidebar_row`: selected symbol
   `ink`, others `ink2`; delete the accent branch. `crates/apps/settings/src/main.rs`: handle
   `WindowEvent::Focus(focused)` for both values and pass it to the view (keep the existing
   `true` work). Test in `crates/apps/settings/tests/settings.rs`: clicking a sidebar row
   focuses it (Up/Down still steps panes) and the rendered scene has no stroke inside the
   sidebar. New golden `about-inactive-light.png`. Regenerate every Settings golden
   (`crates/apps/settings/tests/golden/*.png`: all show a selected row).
3. **Files: same treatment.** Done. `crates/apps/files/src/paint.rs:224-236`: fill from
   `sidebar_selection(self.window_active, false)`, selected symbol `ink`. Add `window_active`
   to the view, set from `WindowEvent::Focus` in `crates/apps/files/src/main.rs`. New golden
   `icons-inactive.png`. Regenerate the Files goldens that show the sidebar
   (`crates/apps/files/tests/golden/*.png`; quick-look ones only if the sidebar is visible).
4. **File chooser: no accent fill in the sidebar.** Done. `crates/toolkit/widgets/src/table.rs`: a
   `Table::sidebar()` style that paints selection with `sidebar_selection(cx.window_active,
   focused)` and keeps `ink` text (`table.rs:1232`, `1418`). Use it in
   `crates/apps/file-chooser/src/view.rs:533`; pass `WindowEvent::Focus` through in
   `crates/apps/file-chooser/src/main.rs`. Test in `crates/toolkit/widgets/tests/table.rs`:
   a focused sidebar table paints no accent fill while a normal one does. New golden
   `open-sidebar-focused.png`; `open.png`, `save.png`, `go-to.png` should stay identical.

Each step is its own branch from `main`, verified on the mini before merging.

## Decisions for the owner
- Keyboard focus in a sidebar: a darker grey step (recommended), or no visible difference
  from plain selection at all?
- The Settings "updates waiting" attention row keeps its accent symbol as a status cue
  (recommended), or goes grey like the rest?

Decided overnight 2026-11-01: keyboard focus in a sidebar shows the darker `hairline` step, no ring; a click also gives the sidebar the keys, so it shows the darker step too (plan recommendation; owner may revisit).
Decided overnight 2026-11-01: the Settings "updates waiting" row keeps its accent symbol as a status cue (plan recommendation; owner may revisit).
