# Plan: launcher recents overlap + Search pane in Settings (search-recents-and-settings)

## (a) Cause: long names in "Recent Documents" overlap
Reproduced headless (launcher scene -> CpuRenderer -> PNG, 6 long names): names without break
opportunities run over the neighbouring cells. `crates/shell/launcher/src/rows.rs::compose_cell`
wraps the title (`paragraph(title, style, Some(width), Center)`, width = cell 86 px - 10) and only
falls back to `fitted()` when `line_count() > CELL_TITLE_LINES`. Parley does not break inside a
word, so `quarterly_financial_report_final_revised_v3.pdf`, `IMG_20261030_142211_HDR_...jpg` or
`verylongfilename...txt` stay at 1-2 lines but are 200-300 px wide; the text starts at the cell's
left and runs to the right. Names with spaces fall into `fitted()` and become one tail-cut line
("Meeting no...", losing the extension). Files already does this right (`files/src/paint.rs
fitted_name`: `shortened(Cut::Middle)` checks `line_count <= lines && width <= width`).

## (a) Fix
Finder/Spotlight: up to two centred lines, middle-truncated so the extension stays.
- `compose_cell`: `shortened(title, cut, |c| p(c, Some(width)).line_count() <= 2 &&
  p.width() <= width + 0.5)` from the toolkit, then draw the wrapped paragraph of the result;
  `Cut::Middle` for Document/Folder hits, `Cut::End` for apps. Same width check in
  `compose_row`'s fallback (Cut::Middle for paths); make the launcher's local `fitted()` a thin
  wrapper over `shortened` (drops ~20 duplicate lines). Size: ~30 lines changed, rows.rs only.

## (b) Search pane (macOS: System Settings > Spotlight)
macOS: checklist of result categories (all on by default) plus "Search Privacy..." with folders
excluded from the index (+ / -). Bande's launcher has these categories (model.rs `rebuild`,
`browse`): Applications, Recent apps, Recent documents, Documents, Folders, System Settings,
System actions (lock/log out/restart), Calculator, Conversion (`calc.rs split_conversion`).
- **Config** (`crates/services/config`): new section, flags omitted when on (default):
  `search { recent-documents #false; folders #false; exclude "~/Private" "~/work/tax" }`.
  `Search` struct with 9 bools + `exclude: Vec<PathBuf>` (~ expanded; reject relative paths),
  parse, `Setting::Search*` edit cases, D-Bus properties `SearchShows` (a{sb}) or one bool per
  category + `SearchExcluded` (as), PropertiesChanged in `publish_changes`; overview.md schema
  line. ~150 lines + ~20 test lines in tests/config.rs.
- **Settings** (`settings-panes/registry.rs` + `apps/settings`): `PaneId::Search` (label "Search",
  symbol `system-search-symbolic`, Desktop group after Notifications, keywords
  spotlight/launcher/recents/privacy). Section "Search results": one `switch` per category,
  described "Choose which results appear when you search with Super+Space". Section "Search
  privacy": `List::SearchExcluded` rows (folder icon, name, path, remove button) + "Add Folder..."
  via the existing portal `choose_file` with `directory: true` (pictures.rs; portal supports it).
  client.rs get/set arms. ~180 lines + locales.
- **Launcher** (`launcher/src/client.rs`, `model.rs`): read `Config` on each `toggle` (like the
  xbel read: one small file, so a change applies on the next open, no bus thread). `Model` gets
  `shown: Search`; `rebuild`/`browse` skip disabled groups before top-hit selection; calculator
  vs conversion split by `split_conversion`. If Documents and Folders are both off, no file
  queries are sent; if Recent documents is off, the xbel is not read. Recents and hits under an
  excluded folder are dropped too (macOS hides them everywhere in Spotlight). ~80 lines.
- **Index** (`libs/search/src/index.rs`): `FileIndex::spawn(root, excluded)`; `indexed()` and
  `read_folder` skip excluded paths; `Command::Exclude(Vec<PathBuf>)` prunes their subtrees
  (`tree.remove`) and crawls re-included folders. Never spawn the index when files are off.
  ~60 lines. Existing skips (dot-folders, node_modules, CACHEDIR.TAG) stay.
Total (b): ~500 lines incl. locales; (a)+(b) ~530.

## Default (recommendation)
All on, exclude list empty: macOS default and today's behaviour; opting out is one click.

## Risks
2-line middle truncation costs ~log2(n) layouts per too-long name (cached by `painted`); exclude
pruning during the first crawl: apply the list before crawling, respawn the index as fallback.

## Verification
desktop-check per step; one gate at the end. Owner on the mini: Super+Space with long recent
names (two centred lines, "...", extension kept, no overlap); Settings > Search: turn off Recent
documents -> next Super+Space has no Recent Documents row; add ~/Private to privacy -> its files
stop appearing. One config test (parse + edit round-trip of `search`); no new e2e test.

## Owner decisions
1. Separate "Recent apps" and "Recent documents" switches (recommended) or one "Recents".
2. Calculator and Conversion as two switches (macOS) or one (recommended: two, it is free).
