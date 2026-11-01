# Ctrl for app shortcuts, Super for the system

## Summary

Today Super does the job of macOS's Command key in Bande's own apps, and Ctrl does nothing
there. Every third-party app the owner uses expects Ctrl. Make Ctrl the app-shortcut key in the
toolkit and in every in-house app, as GNOME and KDE do, and keep Super for the compositor and
shell. The compositor hardly changes, and third-party apps need no change at all.

## What the owner sees

- Copy is `Super+C` in Files, Terminal, Settings and the launcher, but `Ctrl+C` in VS Code,
  Zed, Claude and Firefox. All four are pinned in the bar (`~/.config/bande/config.kdl`).
- In an Bande text field, `Ctrl+C`, `Ctrl+V`, `Ctrl+A` and `Ctrl+Z` do nothing.
  `Ctrl+Left` does not move by a word.
- In lists, Ctrl+click opens the context menu instead of adding the item to the selection.
- The keyboard is a Microsoft PC keyboard, not an Apple one (`/proc/bus/input/devices`:
  "Microsoft Nano Transceiver v2.0" and a Logitech receiver). So the Super key sits where a
  Windows user's thumb never goes for copy and paste.

## Findings

- **This is a design decision, not a bug.** `docs/architecture/overview.md:143-158` (keyboard
  model): "Super acts as macOS's Command" in every app and in the terminal, and Ctrl is left to
  terminals and readline. The decision is repeated in `docs/design/DESIGN.md:30,34-35,68,81`
  and `docs/architecture/toolkit.md:202,233-238`. `docs/architecture/session.md:292-293`
  argues that Super is ⌘ on "the Apple keyboards Bande runs on", which does not match the
  keyboard the owner actually uses.
- **The compositor translates nothing.** `docs/experience/maturity-gaps.md:106` lists this as
  a major gap waiting on an owner decision. Plan item 11 there (`:316-322`) would translate
  Super+key to Ctrl+key, or Ctrl+Shift+key in terminals, for clients that are not Bande's,
  with Firefox's `ui.key.accelKey` as the fallback. The resume notes repeat this as an open
  choice (`NOTES/resume-2026-10-31.md:48`). The owner's note picks neither option. It asks for
  the opposite direction.
- **Compositor** (`crates/platform/compositor/src/input.rs`): `shortcut()` (`:113-189`) only
  runs while Super is held (`:834-856`). The two exceptions are Print and the Mission Control
  key (`without_super`, `:105`). The compositor never takes a Ctrl-only chord, so third-party
  apps already get Ctrl untouched. The shortcuts that close or quit apps from the compositor
  are `Super+W`, `Super+Q`, `Super+M` and `Super+H` (`:155-158,144`). The test
  `every_shortcut_listed_in_settings_runs_its_action` (`:961-973`) ties the compositor to the
  table in Settings.
- **The table in Settings** (`crates/libs/keyboard/src/shortcuts.rs:186-227`, 40 rows) holds
  only system shortcuts, all of them on Super. Settings ▸ Keyboard ▸ Shortcuts renders it
  (`crates/apps/settings/src/pages.rs:884-901`, golden `keyboard-shortcuts-light.png`).
- **Toolkit**:
  - `widgets/src/editor.rs:1313-1411`: every text field. Ctrl returns early at `:1323`.
    Super+Z/X/C/V/A are clipboard and undo, Super+Left/Right go to the line ends, Super+Up/Down
    to the document ends, and Super+Backspace deletes to the line start. Alt+Left/Right moves
    by a word. These are the macOS text rules.
  - `secure_field.rs:193-198`, `table.rs:872-967,1088` (Super+click toggles a row, Super+A,
    Super+C, Super+Up/Down), `menu.rs:1221-1225`, `pop_up_button.rs:173`, `lib.rs:804`.
  - `platform/src/seat.rs:313-314` turns Ctrl+click into a secondary click, as macOS does.
  - `platform/src/window.rs:784-866`: the window menu shows `Ctrl+Super+Left`, `Super+M`,
    `Ctrl+Super+F`, `Super+W` and `Super+Q`.
  - `widgets/src/shortcut.rs` draws shortcuts as macOS symbols (⌃⌥⇧) plus a Super glyph.
- **Shared menu library** (`crates/libs/app-menu/src/lib.rs:100-129`): the standard app-menu
  items Hide `Super+H`, Hide Others `Alt+Super+H`, Quit `Super+Q`, Minimize `Super+M` and Close
  Window `Super+W`. `:221-223` exports the shortcut string as-is, but the dbusmenu spec expects
  `Control`, not `Ctrl`.
- **Apps and shell.** The matching is written by hand in each app. Each also keeps its own copy
  of the menu labels, and nothing checks that the two agree:
  - **Files**:
    - `menu_bar.rs`: 39 shortcuts.
    - `menus.rs`: 21 shortcuts.
    - `view.rs:1190-1234`: Super+click.
    - `view.rs:1492-1560+`: keys, with `let command = modifiers.logo`.
    - `controls.rs:230-231,456`, `grid.rs:223,235` and `quick_look.rs:200`.
    - `menu_bar.rs:104-135` (`key_event`) turns a menu's shortcut string back into a key event.
      It accepts `Control` but not `Ctrl`.
  - **Terminal**:
    - `keys.rs:262-297`: the commands run on Super.
    - `keys.rs:53`: Super is never sent to the PTY.
    - `app.rs:797,1236,1342`: Super+click opens links.
    - `menu.rs`: 23 shortcuts.
  - **Monitor**: `monitor_view.rs:713-773`, `inspector.rs:637-642`, `force_quit.rs:225`,
    `menu.rs` (8 shortcuts).
  - **Settings**: `view.rs:729,736` (Super+F, Super+[).
  - **File chooser**: `view.rs:992,1055`.
  - **Desktop**: `view.rs:679-765,1037`, `menus.rs` (10 shortcuts).
  - **Launcher**: `view.rs:818-859`, `rows.rs:498`.
  - **Screenshot**: `markup_app.rs:151-175`, `picker.rs:291`.
  - **Bar**: `menus.rs:40,248-252`, `menu_search.rs:591`, `dock.rs:2373`.
  - **Strings in the `.ftl` files**:
    - `launcher/locales/.../bande-launcher.ftl:30,33,55,78`: copy and paste hints.
    - `settings/.../bande-settings.ftl:117`: "Go back (Super+[)".
- **Tests and goldens that encode Super**: 23 test files (the toolkit's `text.rs`, `table.rs`,
  `menu.rs`, `ui.rs`, `controls.rs`, `input.rs`, `chrome.rs`; and `files.rs`, `term.rs`,
  `view.rs`, `inspector.rs`, `cascade.rs`, `menu_bar.rs`, `menu_search.rs`, `rail.rs`,
  `desktop.rs`, `launcher.rs`, `export.rs` among others).
  - Goldens that show a shortcut column:
    - toolkit: `menu-submenu*`, `menu-long`, `window-frame-tiling-menu`;
    - bar: `cascade-light*`, `menu-search-light`;
    - compositor: `window-menu*`;
    - Settings: `keyboard-shortcuts-light`;
    - launcher: `launcher-file-light` and others with hints;
    - plus any app context-menu golden.
  - Review scenarios: `ci/review/scenarios/*.scenario` use `super+up` (Files) and `super+1..4`
    (Monitor tabs).
- **References.** GNOME reserves Super for system shortcuts, uses Ctrl as the primary modifier
  (Ctrl+W close, Ctrl+Q quit) and writes labels as text, such as "Shift+Ctrl+S"
  (developer.gnome.org/hig/guidelines/keyboard.html, …/reference/keyboard.html). KDE uses Meta
  for KWin and Plasma global shortcuts and Ctrl in apps. Toshy and Kinto, the Mac-style
  remappers, show how fragile it is to translate Super into Ctrl per app
  (maturity-gaps.md:106).

**Root cause:** Bande copied the macOS modifier model for Apple keyboards, but the owner types
on a PC keyboard and lives in Ctrl-based third-party apps. Since the compositor translates
nothing, two modifier conventions run side by side.

## Options

1. **Ctrl is the app key in Bande's own apps, Super is the system key (GNOME/KDE).**
   - One convention everywhere, and Linux users already know it.
   - Third-party apps already behave this way, so nothing needs translating.
   - Cost: the toolkit, seven apps and the shell need new bindings. The terminal moves to
     Ctrl+Shift+C/V, as GNOME Terminal and Konsole do.
2. **Keep Super as Command and translate Super to Ctrl for third-party clients** (plan item
   11).
   - Mac-like everywhere, and nothing in-house changes.
   - Cost: a compositor rewrite layer with per-app exceptions. Terminals need Ctrl+Shift.
     Apps that bind Super themselves (VS Code, Zed) break. A real Ctrl+C would need its own
     escape hatch. Text navigation (Alt vs Ctrl+arrows) still differs between apps.
   - This is exactly what the owner's note rejects.
3. **Firefox's `ui.key.accelKey` only.** It fixes one app and still depends on Firefox 157
   (bug 2069698). It leaves VS Code, Zed and Claude as they are.
4. **A setting to choose Mac or Linux style.** It needs two sets of bindings, two sets of
   text rules and two sets of goldens for every app. That is speculative surface (CLAUDE.md
   rule 4), and nobody asked for it.

## Recommendation

Option 1. Close the open "Super+key → Ctrl+key or accelKey" decision as "neither". Drop
maturity-gaps plan item 11 and record the reversal in a new ADR,
`docs/decisions/0005-ctrl-for-app-shortcuts.md`.

**Super keeps everything it does today in the compositor and shell**, that is, every row of
`SHORTCUTS`:

- the launcher (`Super+Space`), the switcher (`Super+Tab`, ``Super+` ``), workspaces,
  Overview, tiling, screenshots, panels, Lock, Zoom and Force Quit;
- Hide (`Super+H`) and Minimize (`Super+M`);
- `Super+W` and `Super+Q` as compositor-level close and quit that work in any app.

The compositor also gains `Alt+F4` (close window), the Linux and Windows standard.

**Ctrl takes every app-level shortcut**, using GNOME's bindings rather than a blind swap where
Linux convention differs:

- **Text fields:**
  - Ctrl+C/X/V/A, plus Ctrl+Z, Shift+Ctrl+Z and Ctrl+Y for undo and redo;
  - Ctrl+Left/Right by word, Home/End to the line ends, Ctrl+Home/End to the document ends;
  - Ctrl+Backspace/Delete delete a word;
  - Ctrl+Up/Down by paragraph. Alt+arrows are no longer used in text.
- **Ctrl+click** toggles an item in a selection. The rule that Ctrl+click means right click is
  removed.
- **App standard items:** Close Window `Ctrl+W` and Quit `Ctrl+Q`, handled inside the app.
  Hide and Minimize stay on Super.
- **Files** follows Nautilus where a mechanical swap would read oddly:
  - Move to Trash `Delete`, Delete Immediately `Shift+Delete`;
  - enclosing folder `Alt+Up`, open `Enter`, `Ctrl+O` or `Alt+Down`;
  - back and forward `Alt+Left/Right`;
  - Go to Folder `Ctrl+L`, Show Hidden `Ctrl+H`, Home `Alt+Home`;
  - Add to Sidebar `Ctrl+D`, Quick Look `Space`;
  - previous and next tab `Ctrl+PgUp/PgDn`, as well as the existing Ctrl+Tab.
  - Everything else swaps Super for Ctrl: `Ctrl+N`, `Ctrl+T`, `Shift+Ctrl+N`, `Ctrl+F`, `Ctrl+I`,
    `Ctrl+1..3`, `Ctrl+=`/`Ctrl+-`.
- **Terminal:**
  - every command moves to Shift+Ctrl (C, V, T, N, W, F, G, K, D, comma, Up and Down);
  - zoom is Ctrl+=, Ctrl+- and Ctrl+0;
  - tabs are Ctrl+PgUp/PgDn and Alt+1..9;
  - links open on Ctrl+click;
  - plain Ctrl+letter always goes to the PTY.
- **Monitor, Settings, the launcher, the desktop, the file chooser and screenshot markup:**
  Super becomes Ctrl. Back is Alt+Left.

## Implementation plan

Progress (2026-11-01): milestones 1-10 done. Deviations: the window menus show
Close as `Alt+F4` and Quit as `Super+Q` (both work in every app, Terminal included); Files'
Duplicate has no shortcut (Ctrl+D is Add to Sidebar), Quick Look full screen is F11, Empty Trash
is Shift+Ctrl+Delete and Delete in the Trash deletes immediately; Terminal's find previous is
Shift+Ctrl+H and split stacked Shift+Ctrl+E; Monitor's Quit Process is Ctrl+E. Not written: the
Files every-menu-shortcut test (menu_bar.rs lives in the binary) and app-level tests that Ctrl+W
closes Files/Settings (the routing is in each main.rs); Terminal has the drift test.

Land milestones 2 to 7 back to back, since `main` mixes the two conventions in between. Run
each crate's tests on the mini. Each milestone refreshes its goldens with
`BANDE_UPDATE_GOLDENS=1` and reviews every changed PNG by eye.

1. **One shortcut-string parser** (about 1 h).
   - Move Files' `key_event` parser (`apps/files/src/menu_bar.rs:104-135`) into
     `toolkit/widgets/src/shortcut.rs` as a public function that turns a string into a key and
     modifiers. It accepts `Ctrl` and `Control`.
   - Files calls it.
   - `libs/app-menu` exports `Ctrl` as `Control`, per the dbusmenu spec.
   - Tests: a unit test that parses real menu strings ("Shift+Ctrl+N", "Ctrl+=", "Alt+Up",
     "Shift+Delete") into the matching key and modifiers. Extend `app-menu/tests/export.rs`:
     a `Ctrl+C` item exports `["Control","c"]`.
2. **Toolkit text and widgets to the Linux rules** (about 2 h).
   - Files: `editor.rs:1313-1411` (drop the Ctrl early return, apply the rules above, and the
     shortcut labels in the text context menu), `secure_field.rs`, `table.rs`, `menu.rs`,
     `pop_up_button.rs`, `lib.rs:804`.
   - Remove Ctrl-means-right-click at `platform/src/seat.rs:313`.
   - Tests:
     - `widgets/tests/text.rs`: Ctrl+Left/Right move by word, Home and Ctrl+End,
       Ctrl+Backspace, Ctrl+Z, Shift+Ctrl+Z and Ctrl+Y; Ctrl+V on a read-only field is
       refused; Super+C no longer copies.
     - `table.rs`: Ctrl+click toggles a row, Ctrl+A selects all.
     - `platform/tests/input.rs`: a Ctrl+primary press reaches the app as a primary press
       with Ctrl held.
     - Refresh `text-editing-light-1x`.
3. **Standard app shortcuts and the window menu** (about 1.5 h).
   - `libs/app-menu/src/lib.rs:100-129`: Quit becomes `Ctrl+Q`, Close Window `Ctrl+W`, while
     Hide and Minimize stay on Super.
   - Add `Standard::for_key(key, modifiers)`, which maps Ctrl+W and Ctrl+Q. Files, Settings
     and Monitor call it from their key handlers, in front of their existing `Standard`
     dispatch (`files/src/main.rs:114`, `settings/src/main.rs:94`, `monitor/src/main.rs:103`).
   - The toolkit window menu (`platform/src/window.rs:784-866`) shows `Ctrl+W` and `Ctrl+Q`.
   - Tests:
     - `app-menu` unit test: Ctrl+W maps to CloseWindow, Ctrl+Q to Quit, and Shift+Ctrl+W to
       nothing.
     - A Settings or Files test: Ctrl+W asks the app to close.
     - Refresh `menu-*`, `window-frame-tiling-menu` and `export.rs`.
4. **Compositor: Alt+F4 and the table in Settings** (about 1.5 h).
   - `compositor/src/input.rs`: Alt+F4 closes the focused window outside the Super branch. It
     respects the inhibitor and the lock, like the other shortcuts.
   - `libs/keyboard/src/shortcuts.rs`: add a Close Window row for `Alt+F4`.
   - Change the test at `input.rs:961-973` so a non-Super row is looked up in the non-Super
     table.
   - Tests:
     - The changed test above.
     - A headless test in `tests/headless/keyboard.rs`: Ctrl+W, Ctrl+Q and Ctrl+C reach a plain
       client untouched, and Alt+F4 sends it `xdg_toplevel.close`.
     - Refresh `keyboard-shortcuts-light`.
5. **Files** (about 2 h).
   - Files: `menu_bar.rs`, `menus.rs`, `view.rs`, `controls.rs`, `grid.rs`, `quick_look.rs`,
     using the Nautilus bindings above.
   - Tests:
     - A new `files/tests/files.rs` test: every shortcut in `menu_bar()`, fed through the
       parser from milestone 1 into `View::key`, runs its menu's command. This test is what
       keeps labels and handlers from drifting apart.
     - Update the existing Super tests, including Ctrl+click toggling a row.
     - Update `ci/review/scenarios/files.scenario`.
6. **Terminal** (about 2 h).
   - Files: `keys.rs:53,262-297`, `menu.rs`, `app.rs:797,1236,1342`, `banner.rs:276,319`.
   - Tests in `terminal/tests/term.rs`:
     - Ctrl+C writes 0x03 and Ctrl+W writes 0x17 to the PTY;
     - Shift+Ctrl+C copies the selection, Shift+Ctrl+W closes the tab;
     - every `menu.rs` shortcut parses to the command beside it;
     - Ctrl+click on a link asks xdg-open.
     - Update `terminal.scenario`.
7. **Monitor, Settings, file chooser, desktop, launcher, screenshot, bar** (about 2 h, split it
   if it runs long).
   - Files: those listed under Findings, plus the `.ftl` hints.
   - Tests:
     - Monitor `view.rs` and `inspector.rs`: Ctrl+1 to 4 switch tabs, Ctrl+F searches.
     - `settings/tests/general.rs`: Alt+Left goes back.
     - `launcher.rs`: Ctrl+C copies the result.
     - `desktop.rs`: Ctrl+click toggles.
     - The `file-chooser` view test.
     - Refresh the launcher, cascade and menu-search goldens, and the `monitor-depth` and
       `apps` scenarios.
8. **Shortcut labels as words** (about 1 h, only if the owner agrees).
   - `widgets/src/shortcut.rs` writes "Shift+Ctrl+N" and "Super+Space" in GNOME order (Shift,
     Ctrl, Alt, Super) and drops the ⌃⌥⇧ symbols and the Super glyph.
   - Tests: rewrite `shortcuts_read_as_key_symbols_in_the_standard_modifier_order` for the new
     format and refresh every golden with a shortcut column.
9. **Docs and notes** (about 1.5 h).
   - New ADR `docs/decisions/0005-ctrl-for-app-shortcuts.md`.
   - Rewrite the keyboard model in `overview.md:143-158`.
   - Update:
     - `DESIGN.md:30,34-35,68,81`, `toolkit.md:202,233-238`, `session.md:292-293`;
     - `datum.md:30,251`, `compositor.md:202-204`, `behaviour.md`;
     - `experience/files.md`, `monitor.md`, `settings.md` and `apps-polish.md`.
   - In `maturity-gaps.md`, mark the §4 row Done and remove plan item 11.
   - In `NOTES/resume-2026-10-31.md:48`, mark the decision taken.
   - Update the `docs/design/features/*` texts only where they describe shipped behaviour.
10. **Optional, Linux habits on Super** (about 2 h, by the owner's choice).
    - A tap of Super alone opens the launcher.
    - Alt+Tab works as well as Super+Tab. The switcher commits when the modifier that started
      it is released (`input.rs:879` checks only Super today).
    - Super+Left/Right/Up tile halves and maximize, as in GNOME. They are free now that text
      fields no longer use Super+arrows.
    - Tests: headless `switcher.rs` (Alt+Tab, then releasing Alt, activates the next app),
      `keyboard.rs` (a Super tap opens the launcher; Super+Space and Super+Tab do not also
      count as a tap), `tiling.rs`.

## Risks

- **Terminal regressions.** Ctrl must reach the PTY untouched. Milestone 6's PTY-byte tests
  guard this, and Standard::for_key is not used in Terminal.
- **Menu labels drifting from handlers.** Today nothing ties them together. The
  every-menu-shortcut tests in milestones 5 to 7 fix that per app.
- **Golden churn** across about 15 PNGs. Review them by eye and refresh only the goldens a
  milestone actually changes.
- **Non-Latin layouts.** The toolkit matches on the shifted keysym (`seat.rs:170-177`), so on a
  Cyrillic layout Ctrl+C does not match. This already happens with Super today, so it is not
  a regression, but it shows more now. The fix is the Latin fallback the compositor uses
  (`input.rs:825-830`). Leave it out of scope unless the owner adds a non-Latin source.
- **Muscle memory in the review scenarios and the owner's own habits.** The scenarios change
  in the same commits as the bindings.

## Decisions for the owner

Decided overnight 2026-11-01: shortcut labels read as words, "Ctrl+C" (plan recommendation; owner may revisit).
Decided overnight 2026-11-01: milestone 10, yes to a Super tap, Alt+Tab and Super+arrows (plan recommendation; owner may revisit).
Decided overnight 2026-11-01: Super+W and Super+Q stay as compositor-level close and quit (plan recommendation; owner may revisit).

1. Do shortcuts in menus read as words ("Ctrl+C", the GNOME style, recommended) or as macOS
   symbols with ⌃ standing for Ctrl? This decides whether milestone 8 runs.
2. Should `Super+W` and `Super+Q` stay as compositor-level close and quit alongside
   `Ctrl+W`/`Ctrl+Q` and `Alt+F4`? The recommendation is yes: they are harmless and work in
   apps without Ctrl+Q.
3. Milestone 10: should a tap of Super open the launcher, should Alt+Tab work, and should
   Super+arrows tile as in GNOME? The recommendation is yes to all three, since the owner's
   note asks for "like normal linux distros".
