# Architecture overview

## Goal

Unbound (`unbound-desktop`) is a Linux desktop environment written in Rust for Debian 13 on
arm64, with Apple Silicon as the primary hardware. It is its own desktop: macOS is the bar for
how polished, smooth and finished it feels, and the main reference for which basics a modern
desktop needs, but the structure, feature set and look are ours. The principles that decide
design questions are in [`../design/DESIGN.md`](../design/DESIGN.md).

Everything lives in one Cargo workspace in this repository: compositor, renderer, toolkit, shell
clients, apps, session services, libraries and test harnesses. The layout of that workspace is
recorded in [`../decisions/0003-repository-layout.md`](../decisions/0003-repository-layout.md).
The autonomous software factory that drives development lives in its own repository and is not
described here.

The architecture is split by subject:

| Document | Subject |
| :--- | :--- |
| [`compositor.md`](compositor.md) | the compositor, its renderer, frame pacing and the Wayland protocol matrix |
| [`toolkit.md`](toolkit.md) | the UI toolkit that the shell and the apps draw with |
| [`session.md`](session.md) | session services, D-Bus, the greeter, supervision, setup and updates |
| [`behaviour.md`](behaviour.md) | recorded behaviour decisions and their macOS precedents |
| [`ci.md`](ci.md) | CI runners, jobs and the verification funnel |
| [`packaging.md`](packaging.md) | Debian packages and the meta-package |

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
        RENDER[unbound-render: Vulkan renderer on ash]
        SPRING[unbound-spring: damped springs]
        SMITHAY --> RENDER
    end

    subgraph "Shell clients (started by the compositor)"
        DOCK[Dock]
        MENUBAR[Menu bar]
        LAUNCHER[Launcher]
        CC[Control Center]
        SHOT[Screenshot]
        LOCK[Lock screen]
        GREETER[Greeter]
    end

    subgraph "Apps"
        FILES[Files]
        SETTINGS[Settings]
        TERM[Terminal]
        MONITOR[Activity Monitor]
    end

    subgraph "Session services"
        CONFIG[unbound-config: org.unbound.Config]
        NOTIFY[unbound-notify]
        POWER[unbound-power]
        CLIP[unbound-clipboard]
        TRAY[unbound-tray]
        AUTOMOUNT[unbound-automount]
        PORTAL[xdg-desktop-portal-unbound]
        POLKIT[unbound-polkit]
    end

    TOOLKIT[unbound-toolkit: Vello, Parley, AccessKit, Wayland windows]

    DRM & INPUT & SEAT --> SMITHAY
    DOCK & MENUBAR & LAUNCHER & CC & SHOT & LOCK & GREETER -->|Wayland| SMITHAY
    FILES & SETTINGS & TERM & MONITOR -->|Wayland| SMITHAY
    CLIP -->|Wayland, trusted socket| SMITHAY
    DOCK & MENUBAR & LAUNCHER & FILES & SETTINGS & TERM & MONITOR --- TOOLKIT
    CONFIG & NOTIFY & POWER & TRAY & AUTOMOUNT & PORTAL & POLKIT <-->|D-Bus| SYS
    SETTINGS & MENUBAR & CC <-->|D-Bus| CONFIG
```

The Dock is the crate `unbound-dock` today; [decision 0003](../decisions/0003-repository-layout.md)
renames its directory.

## Process model

Unbound has four kinds of component, split by the role a process plays, not by whether the user
thinks of it as part of the system:

- **Compositor.** One process, `unbound-compositor`, owns the display, input and every Wayland
  surface. It renders with its own renderer (`unbound-render`); title bars and the rest of the
  visible UI are drawn by clients.
- **Shell clients.** Wayland clients that make up the desktop around the apps: Dock, menu bar,
  launcher, Control Center, screenshot tool, lock screen, greeter. The compositor starts them on
  socket pairs and restarts them when they exit; only these connections see the private
  `unbound_shell_v1` protocol and `ext-foreign-toplevel-list-v1`
  ([behaviour](behaviour.md)).
- **Apps.** Ordinary Wayland clients started by the user: Files, Settings, Terminal, Activity
  Monitor. Settings and Activity Monitor are apps like any other; they have no special access.
- **Session services.** Processes without windows that own D-Bus names for the session
  (configuration, notifications, power, clipboard, tray, automount, portal, polkit). They are
  started by systemd user units or D-Bus activation ([session](session.md)).

Shared code that is not UI lives in libraries (`unbound-spring`, `unbound-xdg`,
`unbound-audio`, `unbound-network`, `unbound-bluetooth`, `unbound-protocols`), and the shell
and apps draw with `unbound-toolkit`.

## Inter-process communication

- **Wayland** carries everything about surfaces, buffers, input, focus, clipboard and drag and
  drop. Buffers travel as `wl_shm` or `linux-dmabuf-v1` file descriptors; there is no separate
  data-plane socket. Desktop-specific requests between the compositor and the shell go through
  `unbound_shell_v1` (`crates/platform/protocols`).
- **D-Bus via `zbus`** carries policy, state and desktop integration: the `org.freedesktop.*`
  standards and the `org.unbound.*` interfaces Unbound owns. Every interface can be inspected
  with `busctl`. The matrix is in [session.md](session.md#d-bus-integration).

## Pure Wayland: no X11

Unbound is a Wayland-only desktop by design:

- **Rule**: No X11 code, X libraries, X server or XWayland bridge in any component: compositor,
  shell, toolkit, services or packaging.
- **Not built**: An `xwayland-satellite` bridge (or Smithay's `xwayland`/`xwm` features) is
  deliberately not built. Apps are expected to run natively on Wayland (GTK, Qt, Firefox,
  Chromium, SDL and winit all do).
- **Why**: Every frame, input event and protocol path stays on one modern stack with one
  security model, and nothing in the latency budget pays for X11 translation.

## Keyboard model

Linux conflates clipboard copy with the terminal's `SIGINT` on `Ctrl+C` and works around it
with `Ctrl+Shift+C`. Unbound gives each modifier one meaning across the whole desktop; Super
acts as macOS's Command:

| Modifier | Role | Examples |
| :--- | :--- | :--- |
| **`Super`** | Primary action in every app and the terminal | `Super+C` (copy), `Super+V` (paste), `Super+X` (cut), `Super+Z` (undo), `Super+W` (close), `Super+Q` (quit), `Super+,` (settings) |
| **`Ctrl`** | Terminal signals and readline navigation | `Ctrl+C` (SIGINT), `Ctrl+D` (EOF), `Ctrl+Z` (SIGTSTP), `Ctrl+A` (line start), `Ctrl+E` (line end), `Ctrl+K` (kill to line end) |
| **`Alt`** | Alternate or precise modification | `Alt+Drag` (duplicate, precise move), `Alt+Click` (toggle disclosure), `Alt+Super+Space` (next input source, where macOS uses Control+Space) |
| **`Shift`** | Reversal and extending a selection | `Super+Shift+Z` (redo), `Super+Shift+3/4/5` (screenshots) |

- **Text navigation**: every text field supports `Super+Left/Right` (line start and end) and
  `Alt+Left/Right` (word backward and forward), with `Shift` extending the selection.
- In the terminal, `Super+C` copies and `Super+V` pastes, leaving `Ctrl+C` to interrupt the
  running process.

## Configuration

All user configuration is one human-readable [KDL](https://kdl.dev) file. Every Settings
control writes to it, and it can be edited by hand while the session runs.

- **Design**: node-based, validated against a schema, reloaded live without restarting the
  session. A changed file is parsed into a shadow copy; if it is valid it is swapped in
  atomically and subscribers are told through `org.unbound.Config`; if it is not, the active
  configuration stays as it was and the user is told the line, column and message.
- **Implemented schema (`crates/services/config`)**: KDL 2.0 syntax. The file is
  `$XDG_CONFIG_HOME/unbound/config.kdl`, or `~/.config/unbound/config.kdl` when
  `XDG_CONFIG_HOME` is unset or relative. Every setting is optional; unknown settings,
  duplicates and out-of-range values are rejected with the line and column of the offending
  node or value.
  ```kdl
  appearance "automatic"        // "dark" | "light" | "automatic"
  dock {
      edge "left"               // "left" | "bottom" | "right" | "top"
  }
  idle {
      dim-after "5m"            // positive "<n>s" | "<n>m" | "<n>h"
      lock-after "10m"          // not shorter than dim-after
      suspend-after "10m"       // not shorter than lock-after, or "never"
  }
  keyboard {
      repeat-rate 25            // 1 to 60 characters per second
      repeat-delay 600          // 150 to 2000 milliseconds before repeating starts
      input-sources "se" "us(dvorak)" // xkb layout(variant), at most 4; unset means
                                // the layouts of /etc/default/keyboard
      caps-lock "caps-lock"     // "caps-lock" | "escape" | "control" | "compose" | "off"
  }
  trackpad {
      tap-to-click #true
      natural-scrolling #true
      speed 0.0                 // -1.0 to 1.0
      secondary-click "two-fingers" // "two-fingers" | "corner"
      disable-while-typing #true
  }
  mouse {
      natural-scrolling #false
      speed 0.0                 // -1.0 to 1.0
  }
  sound {
      effects #true             // screenshot, Trash, power and battery sounds
      volume-feedback #true     // a tick on each volume key press
      alert "mallet"            // "mallet" | "knock" | "low" | "tick"
      alert-volume 0.75         // 0.0 to 1.0, scales every event sound
  }
  wallpaper {                   // absolute path or "~/..."; unset means the built-in picture
      light "/usr/share/backgrounds/unbound/day.jpg"
      dark "~/Pictures/night.jpg"
  }
  output "eDP-1" {              // once per connector
      scale 1.5                 // 1.0 to 2.0 (100%-200%)
      refresh-rate 120          // 60 to 240 Hz
      variable-refresh-rate #true
  }
  ```
  The values shown for `appearance`, `dock`, `idle`, `keyboard` (except `input-sources`), `trackpad`, `mouse` and `sound` are the defaults. The
  wallpaper used is the one matching the current appearance; the desktop expands a leading `~/`.
  A section left empty by a Settings write is removed from the file. A valid file is parsed
  into a shadow `Config` and swapped in as one `Arc`, so readers see either the old or the new
  configuration, never a mix. `ConfigService` owns `org.unbound.Config` on the session bus and
  serves the active values as properties of interface `org.unbound.Config` at
  `/org/unbound/Config` (`Appearance`, `DockEdge`, `IdleDimAfter`, `IdleLockAfter` and
  `IdleSuspendAfter` in seconds, 0 meaning never, `IdleSuspendWithDisplay` true while
  `suspend-after` is unset and sleep follows the display (writing true removes the explicit
  value), `KeyRepeatRate` per second and
  `KeyRepeatDelay` in milliseconds as `u`, `InputSources` as `as` of layout codes (the system layouts while unset; writing an empty list is rejected), `CapsLock`, `TrackpadTapToClick`, `TrackpadNaturalScrolling`, `TrackpadSpeed` (`d`), `TrackpadSecondaryClick`, `TrackpadDisableWhileTyping`, `MouseNaturalScrolling` and `MouseSpeed` (`d`), `WallpaperLight` and `WallpaperDark` as paths, empty
  meaning unset, `Outputs` as `a{sa{sv}}`). It watches the
  directory and reloads when the file is closed after writing or renamed into place;
  `PropertiesChanged` lists only the properties that changed. The `unbound-config` daemon runs
  the service for the session and posts an invalid file as a notification through
  `unbound-notify` with the line, column and message and an "Open in Editor" action
  (`xdg-open` on the file); a later error replaces it and a valid file closes it. The
  compositor does not draw a toast of its own. The scalar properties are writable and
  `SetOutput(s connector, a{sv} settings)` replaces one output's settings (an empty dictionary
  removes it): a write edits only the affected node of the file, keeping the user's comments
  and layout, validates the whole result, replaces the file by rename and applies it, so the
  file stays the single source of truth. A value the schema rejects fails with `InvalidArgs`,
  and a file that is invalid on disk is never overwritten.
