# Architecture overview

## Goal

Bande (`bande-desktop`) is a Linux desktop environment written in Rust for Debian testing on
arm64 and amd64, with Apple Silicon as the primary hardware. It is its own desktop: macOS is the bar for
how polished, smooth and finished it feels, and the main reference for which basics a modern
desktop needs, but the structure, feature set and look are ours. The principles that decide
design questions are in [`../design/DESIGN.md`](../design/DESIGN.md).

The desktop lives in one Cargo workspace in this repository: compositor, renderer, toolkit,
shell clients, session services, Files, Settings, libraries and test harnesses. Its layout is
recorded in [`../decisions/0003-repository-layout.md`](../decisions/0003-repository-layout.md).
The other Bande apps (Terminal, Activity Monitor, Software, the Package Installer, Mail,
Calendar) live in [bande-linux/apps](https://github.com/bande-linux/apps), built on the same
toolkit and app framework ([apps.md](apps.md)).

The architecture is split by subject:

| Document | Subject |
| :--- | :--- |
| [`compositor.md`](compositor.md) | the compositor, its renderer, frame pacing and the Wayland protocol matrix |
| [`toolkit.md`](toolkit.md) | the UI toolkit that the shell and the apps draw with |
| [`session.md`](session.md) | session services, D-Bus, the greeter, supervision, setup and updates |
| [`behaviour.md`](behaviour.md) | recorded behaviour decisions and their macOS precedents |
| [`apps.md`](apps.md) | the app framework every app is built on |
| [`ci.md`](ci.md) | the local gate, the release workflow and how a release is cut |
| [`packaging.md`](packaging.md) | Debian packages and the meta-package |
| [`performance.md`](performance.md) | how to measure memory, CPU and wake-ups, and the current numbers |

## Component map

```mermaid
graph TB
    subgraph "Kernel and system"
        DRM[DRM/KMS, GBM, dma-buf]
        INPUT[libinput]
        SEAT[libseat / systemd-logind]
        SYS["System services: logind, UPower, NetworkManager, BlueZ, UDisks2, polkitd, PipeWire"]
    end

    subgraph "Compositor process"
        SMITHAY[Smithay: DRM backend, input, Wayland protocols]
        RENDER[bande-render: Vulkan renderer on ash]
        SPRING[bande-spring: damped springs]
        FQ[Force Quit window]
        SMITHAY --> RENDER
    end

    subgraph "Shell clients (started by the compositor)"
        subgraph "bande-shell"
            DESKTOP[Desktop]
            DOCK[Taskbar]
            MENUBAR[Menu bar]
            NOTES[Notifications]
            LAUNCHER[Launcher]
            CC[Control Center]
        end
        SHOT[Screenshot]
        LOCK[Lock screen]
        GREETER[Greeter]
    end

    subgraph "Apps (bande-app)"
        FILES[Files]
        SETTINGS[Settings]
        TERM["Terminal, Activity Monitor, Software, ... (bande-linux/apps)"]
    end

    subgraph "Session services"
        CONFIG[bande-config: org.bande.Config]
        NOTIFY[bande-notify]
        POWER[bande-power]
        SOUND[bande-sound]
        CLIP[bande-clipboard]
        TRAY[bande-tray]
        AUTOMOUNT[bande-automount]
        PORTAL[xdg-desktop-portal-bande]
        POLKIT[bande-polkit]
    end

    TOOLKIT[bande-toolkit: Vello, Parley, AccessKit, Wayland windows]

    DRM & INPUT & SEAT --> SMITHAY
    DOCK & MENUBAR & LAUNCHER & CC & SHOT & LOCK & GREETER -->|Wayland| SMITHAY
    FILES & SETTINGS & TERM -->|Wayland| SMITHAY
    CLIP -->|Wayland, trusted socket| SMITHAY
    DOCK & MENUBAR & LAUNCHER & FILES & SETTINGS & TERM --- TOOLKIT
    CONFIG & NOTIFY & POWER & TRAY & AUTOMOUNT & PORTAL & POLKIT <-->|D-Bus| SYS
    SETTINGS & MENUBAR & CC <-->|D-Bus| CONFIG
```

The desktop and the taskbar of every display (with the menu bar when it is separate), the notifications, Control Center and the
launcher are one process, `bande-shell` (`crates/shell/shell`), with one Wayland connection, one
GPU device, one font database and one connection per D-Bus bus. Each part is a library crate
(`bande-desktop-surface`, `bande-bar`, `bande-notifications`, `bande-controlcenter`,
`bande-launcher`) whose surfaces the shell hosts; the taskbar's models live in
`bande-taskbar-model` and `bande-status` (`crates/libs`).

## Process model

Bande has four kinds of component, split by the role a process plays, not by whether the user
thinks of it as part of the system:

- **Compositor.** One process, `bande-compositor`, owns the display, input and every Wayland
  surface. It renders with its own renderer (`bande-render`) and draws what must work when
  every client hangs: Force Quit, the window menu, the switcher and overview, and title bars
  for apps that ask for server-side ones. Everything else is drawn by clients.
- **Shell clients.** Wayland clients that make up the desktop around the apps: the shell (desktop,
  taskbar, menu bar, notifications, launcher, Control Center), the screenshot tool, the lock screen
  and the greeter. The compositor starts them on
  socket pairs and restarts them when they exit; only these connections see the private
  `bande_shell_v1` protocol and `ext-foreign-toplevel-list-v1`
  ([behaviour](behaviour.md)).
- **Apps.** Ordinary Wayland clients started by the user, each one process for all its windows
  on `bande-app` ([apps](apps.md)): Files and Settings here, the rest in bande-linux/apps.
  Settings and Activity Monitor have no special access. Closing an app's last window leaves it
  running until Quit, as on macOS.
- **Session services.** Processes without windows that own D-Bus names for the session
  (configuration, notifications, power, sound, clipboard, tray, automount, portal, polkit). They are
  started by systemd user units or D-Bus activation ([session](session.md)).

Shared code that is not UI lives in `crates/libs` (`bande-xdg`, `bande-fs`, `bande-audio`,
`bande-network`, `bande-bluetooth`, `bande-printers`, `bande-packages`, `bande-spring` and more), and the shell
and apps draw with `bande-toolkit`.

## Inter-process communication

- **Wayland** carries everything about surfaces, buffers, input, focus, clipboard and drag and
  drop. Buffers travel as `wl_shm` or `linux-dmabuf-v1` file descriptors; there is no separate
  data-plane socket. Desktop-specific requests between the compositor and the shell go through
  `bande_shell_v1` (`crates/platform/protocols`).
- **D-Bus via `zbus`** carries policy, state and desktop integration: the `org.freedesktop.*`
  standards and the `org.bande.*` interfaces Bande owns. Every interface can be inspected
  with `busctl`. The matrix is in [session.md](session.md#d-bus-integration).

## Pure Wayland: no X11

Bande is a Wayland-only desktop by design:

- **Rule**: No X11 code, X libraries, X server or XWayland bridge in any component: compositor,
  shell, toolkit, services or packaging.
- **Not built**: An `xwayland-satellite` bridge (or Smithay's `xwayland`/`xwm` features) is
  deliberately not built. Apps are expected to run natively on Wayland (GTK, Qt, Firefox,
  Chromium, SDL and winit all do).
- **Why**: Every frame, input event and protocol path stays on one modern stack with one
  security model, and nothing in the latency budget pays for X11 translation.

## Keyboard model

Bande follows the Linux convention of GNOME and KDE: Ctrl runs shortcuts inside apps and Super
belongs to the system ([decision 0005](../decisions/0005-ctrl-for-app-shortcuts.md)). Bande's
own apps and third-party apps use the same keys, so the compositor translates nothing.

| Modifier | Role | Examples |
| :--- | :--- | :--- |
| **`Ctrl`** | Shortcuts inside apps and text fields | `Ctrl+C` (copy), `Ctrl+V` (paste), `Ctrl+X` (cut), `Ctrl+Z` (undo), `Ctrl+W` (close window), `Ctrl+Q` (quit), `Ctrl+F` (find), `Ctrl+click` (toggle an item in a selection) |
| **`Super`** | The system only, never an app action | a tap of `Super`, `Super+Space` or `Super+A` (launcher), `Super+Tab` (switcher), `Super+arrows` (tiling), `Super+H` (minimize), `Super+L` (lock), `Super+D` (desktop), `Super+W` (overview), `Super+V` (clipboard history), `Super+.` (emoji), `Super+N` and `Super+S` (notifications, Control Center), `Ctrl+Super+Left/Right` (workspaces), Super+drag (move a window) |
| **`Alt`** | Navigation in apps, alternate actions | `Alt+Left/Right` (back and forward), `Alt+Up` (enclosing folder), `Alt+F4` (close window), `Alt+Tab` (switcher), `Alt+Click` (toggle disclosure) |
| **`Shift`** | Reversal, extending a selection, the second set of commands | `Shift+Ctrl+Z` (redo), `Shift+Ctrl+N` (new folder), Terminal's `Shift+Ctrl+C/V`, `Shift+Drag` (move) |

- **Screenshots and Force Quit** need no Super: `Print` (options), `Shift+Print` (the screen),
  `Shift+Super+S` (an area), `Ctrl+Alt+Escape` (Force Quit). Settings ▸ Keyboard ▸ Shortcuts
  lists the whole system set from the table in `bande-keyboard`.
- **Drag and drop**: `Ctrl` copies and `Shift` moves, as in GTK and KDE.

- **Text fields**: `Ctrl+Left/Right` move by word, `Home`/`End` to the line ends,
  `Ctrl+Home/End` to the text ends, `Ctrl+Up/Down` by paragraph, and `Ctrl+Backspace/Delete`
  delete a word; `Shift` extends the selection. `Ctrl+Y` also redoes. Alt and Super are not used
  in text fields.
- **Terminal**: every plain `Ctrl+letter` goes to the shell (`Ctrl+C` interrupts, `Ctrl+D` ends
  input), so the Terminal's own commands are on `Shift+Ctrl`, as in GNOME Terminal and Konsole.
- **Menus** write shortcuts as words in GNOME order (Shift, Ctrl, Alt, Super): "Shift+Ctrl+N",
  "Alt+Super+8".

## Configuration

All user configuration of the desktop is one human-readable [KDL](https://kdl.dev) file,
`$XDG_CONFIG_HOME/bande/config.kdl` (`~/.config/bande/config.kdl`). Every Settings control writes
to it, and it can be edited by hand while the session runs. Apps keep their own settings in one
small `key = value` file each next to it (`files.conf`, `terminal.conf`).

- **Schema** (`crates/services/config`, KDL 2.0): top-level `appearance`, `accent`,
  `tinted-frame`, `clock`, `reopen-apps`, `reopen-apps-after-crash`, and the sections
  `accessibility`, `bar`, `idle`, `keyboard`, `trackpad`, `mouse`, `power`, `sound`, `wallpaper`,
  `night-light`, `do-not-disturb`, `search`, `display "<name>"` (one per display, named by EDID maker, model and serial) and
  `arrangement` (one per set of displays). Every setting is optional and has a default.
  ```kdl
  appearance "automatic"        // "dark" | "light" | "automatic"
  bar { edge "left"; size 40; auto-hide #false; menu-bar "in-bar" }
  idle { dim-after "5m"; lock-after "10m" }   // suspend-after follows lock-after unless set
  keyboard { input-sources "se" "us(dvorak)"; caps-lock "escape" }
  trackpad { tap-to-click #true; natural-scrolling #true }
  search { recent-documents #false; exclude "~/Private" }   // every category is shown unless #false
  display "Dell DELL U2720Q 7ZD5K53" { scale 1.5; refresh-rate 120 }
  arrangement {
      display "BOE NE135A1M" x=0 y=0 main=#true
      display "Dell DELL U2720Q 7ZD5K53" x=1504 y=-300
  }
  ```
- **Errors**: an unknown setting, a duplicate or an out-of-range value is skipped and reported
  with its line and column while every other setting applies; a file that is not KDL keeps the
  active configuration. `bande-config` posts the errors as one notification with an "Open in
  Editor" action, once per distinct set of errors.
- **Live reload**: the service watches the directory, parses a changed file into a shadow copy
  and swaps it in atomically, so readers never see a mix.
- **`org.bande.Config`** (`/org/bande/Config`, session bus) serves every value as a property and
  signals only the properties that changed. The scalar properties are writable, and
  `SetDisplay` and `SetArrangement` replace one display's settings or one arrangement. A write
  edits only the affected node, keeping the user's comments and layout, validates the whole
  result and replaces the file by rename; a value the schema rejects fails with `InvalidArgs`,
  and a file that is not KDL is never overwritten.
