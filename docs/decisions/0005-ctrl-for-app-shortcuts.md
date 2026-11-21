# Ctrl for App Shortcuts: Decision Record

Status: **decided** (2026-11-01). Scope: which modifier runs shortcuts in Bande's own apps, text
fields and shell, and which is left to the system.

## Context

- Apps on a Linux desktop run their shortcuts with Ctrl, and the compositor translates no keys,
  so Bande's own apps use Ctrl too, and one set of habits works in every app.
- The system needs chords no app takes: the compositor takes chords with Super held, plus
  Print, `Alt+Tab`, `Alt+F4` and a few `Ctrl+Alt` chords, so apps get their Ctrl chords
  untouched.

## Options

1. **Ctrl is the app key, Super is the system key.** One convention in every app; nothing to
   translate.
2. **A setting for two styles.** Two sets of bindings and text rules in every app, for a choice
   that buys the user nothing.

## Decision

Option 1. Ctrl runs app shortcuts; Super belongs to the compositor and the shell.

- **Super is only for the system**, and nothing an app does (close, quit, copy, paste, save,
  find, tabs) is on Super. The set is `SHORTCUTS` in `crates/libs/keyboard/src/shortcuts.rs`, the
  one table the compositor matches and Settings ▸ Keyboard lists: Search on a tap of Super;
  `Super+V`, `Super+.`, `Super+N`, `Super+S`, `Super+L`; `Alt+F4`, `Super+H`, `Super+arrows`,
  `Shift+Super+arrows`; `Alt+Tab` (the only switcher), `Ctrl+Alt+T` Terminal;
  `Ctrl+Super+Left/Right`, `Super+W` (Overview), `Super+D` (Show Desktop); `Print`,
  `Shift+Print`, `Shift+Super+S`; `Ctrl+Alt+Esc` (Force Quit); `Ctrl+Alt+F1…F12`; the zoom and
  screen reader keys. Input sources, app menus and full screen have no system shortcut.
  Super+drag moves a window and Super+right drag resizes it.

  Unbound Super chords reach the focused app.
- **Text fields** follow the Linux rules: `Ctrl+C/X/V/A`; `Ctrl+Z` undo, `Shift+Ctrl+Z` and
  `Ctrl+Y` redo; `Ctrl+Left/Right` by word, `Home`/`End` to the line ends, `Ctrl+Home/End` to the
  text ends, `Ctrl+Up/Down` by paragraph; `Ctrl+Backspace/Delete` delete a word. Alt and Super are
  not used in text fields.
- **Ctrl+click** toggles an item in a selection.
- **App standard items**: Close Window `Ctrl+W` and Quit `Ctrl+Q`, handled inside the app.
  Minimize is `Super+H`; `F11` enters and leaves full screen (Window menu, not panels); `Ctrl+Y`
  runs any `Shift+Ctrl+Z` redo item. Hide and Hide Other Apps stay in the app menu without a chord.
- **Apps take the usual Linux bindings** beyond the standard items: Files has
  `Delete` to the Trash, `Alt+Up` to the parent folder, `Alt+Left/Right` back and
  forward, `Ctrl+L` Go to Location, `Ctrl+H` hidden files, `F2` rename; Terminal puts its commands
  on `Shift+Ctrl` and sends every plain Ctrl+letter to the shell; Settings goes back with
  `Alt+Left`.
- **Shortcut labels are words** in the order Shift, Ctrl, Alt, Super: "Shift+Ctrl+N",
  "Alt+Super+8", "Page Up", "Esc", arrows as arrow symbols; no modifier symbols.

## Consequences

- Shortcuts learned in any Linux app work the same in Bande's apps, and the reverse.
- In drag and drop, Ctrl copies and Shift moves.
- One parser in `widgets/src/shortcut.rs` reads a menu's shortcut string ("Shift+Ctrl+N") back
  into a key and modifiers; `bande-app` runs the menu entry for a key the view did not take, so
  an app's menus are its shortcut table ([apps](../architecture/apps.md)).
