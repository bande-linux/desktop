# Ctrl for App Shortcuts: Decision Record

Status: **decided** (2026-11-01). Scope: which modifier runs shortcuts in Bande's own apps, text
fields and shell, and which is left to the system.

## Context

- On a PC keyboard Super sits where no thumb goes for copy and paste.
- Third-party apps (VS Code, Zed, Firefox) expect Ctrl, and the compositor translates nothing, so
  Bande's own apps must use the same convention for one set of habits to work everywhere.
- The compositor takes chords with Super held, plus Print, `Alt+Tab`, `Alt+F4` and a few
  `Ctrl+Alt` chords, so third-party apps get Ctrl chords untouched.

## Options

1. **Ctrl is the app key, Super is the system key**, the convention of the Linux desktops. One
   convention in every app; nothing to translate.
2. **Super as the app key, translated to Ctrl for other clients.** It needs a compositor rewrite
   layer with per-app exceptions, Ctrl+Shift for terminals and an escape hatch for a real Ctrl+C,
   and it breaks apps that bind Super themselves. Text navigation still differs between apps.
3. **Firefox's `ui.key.accelKey` only.** Fixes one app and leaves the rest.
4. **A setting for two styles.** Two sets of bindings and text rules in every app, for a choice
   nobody asked for.

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
- **Apps take the usual Linux bindings** where they differ from a plain swap: Files has
  `Delete` to the Trash, `Alt+Up` to the enclosing folder, `Alt+Left/Right` back and
  forward, `Ctrl+L` Go to Location, `Ctrl+H` hidden files, `F2` rename; Terminal puts its commands
  on `Shift+Ctrl` and sends every plain Ctrl+letter to the shell; Settings goes back with
  `Alt+Left`.
- **Shortcut labels are words** in the order Shift, Ctrl, Alt, Super: "Shift+Ctrl+N",
  "Alt+Super+8", "Page Up", "Esc", arrows as arrow symbols; no modifier symbols.

## Consequences

- Muscle memory from VS Code, Firefox and the terminal works the same in Bande's apps.
- In drag and drop, Ctrl copies and Shift moves.
- One parser in `widgets/src/shortcut.rs` reads a menu's shortcut string ("Shift+Ctrl+N") back
  into a key and modifiers; `bande-app` runs the menu entry for a key the view did not take, so
  an app's menus are its shortcut table ([apps](../architecture/apps.md)).

## Sources

1. GNOME Human Interface Guidelines, Keyboard: developer.gnome.org/hig/guidelines/keyboard.html
   and developer.gnome.org/hig/reference/keyboard.html.
2. KDE: Meta for KWin and Plasma global shortcuts, Ctrl in apps (KDE HIG, keyboard shortcuts).
