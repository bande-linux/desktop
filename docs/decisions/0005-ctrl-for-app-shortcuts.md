# Ctrl for App Shortcuts: Decision Record

Status: **decided** (2026-11-01, amended the same day after the owner's review). Scope: which modifier runs shortcuts in Bande's own apps, text
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

- **Super is only for the system**, in GNOME and KDE's chords, and nothing an app does (close,
  quit, copy, paste, save, find, tabs) is on Super. The set, every row of the table in
  `bande-keyboard`:
  - launcher: a tap of Super, `Super+Space` (kept by the owner) and `Super+A`; clipboard history
    `Super+V`; emoji `Super+.`; Notification Center `Super+N`; Control Center `Super+S`;
    lock `Super+L`; menu bar and taskbar `Super+F2`/`Super+F3`; menu search `Shift+Super+/`;
  - windows: `Alt+F4` close, `Super+H` minimize, `Super+Left/Right/Up/Down` tile, maximize and
    restore, `Super+F11` full screen, `Shift+Super+arrows` to the display beside, Super+drag
    moves and Super+right drag resizes;
  - apps: `Super+Tab` and `Alt+Tab` switch, ``Super+` `` the app's next window,
    `Ctrl+Alt+Escape` Force Quit;
  - workspaces: `Ctrl+Super+Left/Right`, `Ctrl+Super+1..9`, `Ctrl+Shift+Super+Left/Right`
    carries the window, `Super+W` the overview, `Alt+Super+Down` the app's windows, `Super+D`
    the desktop;
  - input sources `Alt+Super+Space`; screenshots `Print` (options), `Shift+Print` (screen),
    `Shift+Super+S` (area); zoom `Alt+Super+8`, `Alt+Super+=`/`-`; screen reader `Alt+Super+S`.

  Unbound Super chords reach the focused app, as in GNOME.
- **Text fields** follow the Linux rules: `Ctrl+C/X/V/A`; `Ctrl+Z` undo, `Shift+Ctrl+Z` and
  `Ctrl+Y` redo; `Ctrl+Left/Right` by word, `Home`/`End` to the line ends, `Ctrl+Home/End` to the
  text ends, `Ctrl+Up/Down` by paragraph; `Ctrl+Backspace/Delete` delete a word. Alt and Super are
  not used in text fields.
- **Ctrl+click** toggles an item in a selection. It is no longer a right click.
- **App standard items**: Close Window `Ctrl+W` and Quit `Ctrl+Q`, handled inside the app.
  Minimize is `Super+H`; Hide and Hide Others stay in the app menu without a chord.
- **Apps take GNOME's bindings** where Linux convention differs from a plain swap: Files follows
  Nautilus (`Delete` to the Trash, `Alt+Up` to the enclosing folder, `Alt+Left/Right` back and
  forward, `Ctrl+L` Go to Folder, `Ctrl+H` hidden files, `F2` rename); Terminal puts its commands
  on `Shift+Ctrl` and sends every plain Ctrl+letter to the shell; Settings goes back with
  `Alt+Left`.
- **Shortcut labels are words** in GNOME order (Shift, Ctrl, Alt, Super): "Shift+Ctrl+N",
  "Alt+Super+8", "Page Up", "Esc", arrows as arrow symbols. The ⌃⌥⇧ symbols and the Super glyph
  are gone.

## Consequences

- Muscle memory from VS Code, Firefox and the terminal works the same in Bande's apps.
- Drag and drop takes GTK and KDE's modifiers: Ctrl copies and Shift moves.
- One parser in `widgets/src/shortcut.rs` reads a menu's shortcut string ("Shift+Ctrl+N") back
  into a key and modifiers. Terminal's menu labels are tested against its key handling with it;
  in the other apps a label and its handler are still written separately and must be changed
  together.
- On a non-Latin layout Ctrl+C does not match yet, because the toolkit matches the shifted
  keysym. This held for Super before, so it is not a regression; the compositor's Latin fallback
  is the fix when a non-Latin source is added.
- The first version of this decision kept the macOS system chords on Super (`Super+W` close,
  `Super+Q` quit, `Super+H` hide, `Super+M` minimize, `Ctrl+Super+Q` lock, `Shift+Super+3/4/5`
  screenshots, `Ctrl+Super+Space` emoji, `Alt+Super+Escape` Force Quit). The owner found that
  Super still ran a lot of shortcuts, so the amendment above replaced them with the GNOME and
  KDE chords (`NOTES/plans/super-ctrl.md`, Follow-up 2026-11-01).

## Sources

1. GNOME Human Interface Guidelines, Keyboard: developer.gnome.org/hig/guidelines/keyboard.html
   and developer.gnome.org/hig/reference/keyboard.html.
2. KDE: Meta for KWin and Plasma global shortcuts, Ctrl in apps (KDE HIG, keyboard shortcuts).
3. The plan and findings: `NOTES/plans/super-ctrl.md`.
