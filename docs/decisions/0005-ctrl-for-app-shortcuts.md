# Ctrl for App Shortcuts: Decision Record

Status: **decided** (2026-11-01). Scope: which modifier runs shortcuts in Bande's own apps, text
fields and shell, and which is left to the system. Replaces the "Super acts as Command" keyboard
model of the [overview](../architecture/overview.md#keyboard-model).

## Context

- Until now Super did the job of macOS's Command key in Bande's own apps: `Super+C` copied in
  Files, Terminal, Settings and the launcher, text fields followed the macOS text rules
  (`Super+Left/Right` to the line ends, `Alt+Left/Right` by word), Ctrl did nothing there, and
  Ctrl+click was a right click. The idea was one meaning per modifier on the Apple keyboards
  Bande was first planned for.
- The owner types on a PC keyboard (a Microsoft and a Logitech receiver), where Super sits where
  no thumb goes for copy and paste.
- Every third-party app the owner uses (VS Code, Zed, Claude, Firefox, all pinned in the bar)
  expects Ctrl. The compositor translates nothing, so two conventions ran side by side, and
  `Ctrl+C` in a Bande text field did nothing.
- The compositor only takes chords with Super held (plus Print and the Mission Control key), so
  third-party apps already get Ctrl untouched.

## Options

1. **Ctrl is the app key, Super is the system key**, as GNOME and KDE do. One convention in every
   app; nothing to translate. Cost: new bindings in the toolkit, seven apps and the shell.
2. **Keep Super as Command and translate Super to Ctrl for other clients** (maturity-gaps plan
   item 11). Mac-like everywhere, but it needs a compositor rewrite layer with per-app
   exceptions, Ctrl+Shift for terminals, an escape hatch for a real Ctrl+C, and it breaks apps
   that bind Super themselves. Text navigation still differs between apps. The owner's note asks
   for the opposite.
3. **Firefox's `ui.key.accelKey` only.** Fixes one app and leaves the rest.
4. **A setting for Mac or Linux style.** Two sets of bindings, text rules and goldens in every
   app, for a choice nobody asked for.

## Decision

Option 1. Ctrl runs app shortcuts; Super belongs to the compositor and the shell.

- **Super keeps every system shortcut**, every row of the table in `bande-keyboard`: the launcher
  (`Super+Space`), the switcher, workspaces, the overview, tiling, screenshots
  (`Shift+Super+3/4/5`), panels, lock (`Ctrl+Super+Q`), Hide (`Super+H`), Minimize (`Super+M`),
  and `Super+W` and `Super+Q` as close and quit that work in any app. `Alt+F4` also closes the
  focused window.
- **Text fields** follow the Linux rules: `Ctrl+C/X/V/A`; `Ctrl+Z` undo, `Shift+Ctrl+Z` and
  `Ctrl+Y` redo; `Ctrl+Left/Right` by word, `Home`/`End` to the line ends, `Ctrl+Home/End` to the
  text ends, `Ctrl+Up/Down` by paragraph; `Ctrl+Backspace/Delete` delete a word. Alt and Super are
  not used in text fields.
- **Ctrl+click** toggles an item in a selection. It is no longer a right click.
- **App standard items**: Close Window `Ctrl+W` and Quit `Ctrl+Q`, handled inside the app. Hide
  and Minimize stay on Super.
- **Apps take GNOME's bindings** where Linux convention differs from a plain swap: Files follows
  Nautilus (`Delete` to the Trash, `Alt+Up` to the enclosing folder, `Alt+Left/Right` back and
  forward, `Ctrl+L` Go to Folder, `Ctrl+H` hidden files, `F2` rename); Terminal puts its commands
  on `Shift+Ctrl` and sends every plain Ctrl+letter to the shell; Settings goes back with
  `Alt+Left`.
- **Shortcut labels are words** in GNOME order (Shift, Ctrl, Alt, Super): "Shift+Ctrl+N",
  "Alt+Super+H", "Page Up", "Esc", arrows as arrow symbols. The ⌃⌥⇧ symbols and the Super glyph
  are gone.

## Consequences

- Muscle memory from VS Code, Firefox and the terminal works the same in Bande's apps.
- Drag and drop keeps its modifiers: Alt copies and Super moves, as Option and Command do on
  macOS. Ctrl does not change a drop.
- One parser in `widgets/src/shortcut.rs` reads a menu's shortcut string ("Shift+Ctrl+N") back
  into a key and modifiers. Terminal's menu labels are tested against its key handling with it;
  in the other apps a label and its handler are still written separately and must be changed
  together.
- On a non-Latin layout Ctrl+C does not match yet, because the toolkit matches the shifted
  keysym. This held for Super before, so it is not a regression; the compositor's Latin fallback
  is the fix when a non-Latin source is added.
- Open, by the owner's choice: a tap of Super alone opening the launcher, `Alt+Tab` beside
  `Super+Tab`, and `Super+Left/Right/Up` tiling as in GNOME. Super+arrows are free now that text
  fields no longer use them.

## Sources

1. GNOME Human Interface Guidelines, Keyboard: developer.gnome.org/hig/guidelines/keyboard.html
   and developer.gnome.org/hig/reference/keyboard.html.
2. KDE: Meta for KWin and Plasma global shortcuts, Ctrl in apps (KDE HIG, keyboard shortcuts).
3. The plan and findings: `NOTES/plans/super-ctrl.md`.
