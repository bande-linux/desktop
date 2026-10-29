# Feature catalogue

The first curated feature list, drawn from macOS, GNOME, KDE and COSMIC before the design
principles existed. It is kept for reference: the proposals in [`features/`](features/)
supersede it area by area and are measured against [`DESIGN.md`](DESIGN.md), and where they
disagree, the proposal and the owner's decision on it win. Section numbers are cited by the
proposals.

## 1. `unbound-files` (Miller Columns File Manager)
- **Miller Columns**: Multi-pane navigation with arrow keys (`Right` drills into child folder in new column, `Left` retreats, `Up`/`Down` selects).
- **Quick Look**: Pressing `Spacebar` summons an instant overlay rendering images, videos, markdown, PDFs, or syntax-highlighted code.
- **Side-Pane Inspector**: Displays EXIF data, permissions, MIME type, file size, SHA-256 hash, and audio metadata.
- **Productivity Tools**: Batch renaming engine (regex, find-and-replace, numbering), breadcrumb path bar with drag-and-drop.
- **Undo Stack (`Super+Z`)**: Multi-level undo for file operations (renames, folder moves, trashing) via XDG Trash specification (`~/.local/share/Trash`) with "Put Back" restoring files to their original paths.

## 2. `unbound-settings` (Unified System Preferences)
- Modular KDL-backed settings categorized into logical panes:
  - **Displays**: Resolution, fractional scale (100%–200%), refresh rate (60Hz–240Hz), VRR/FreeSync, visual multi-monitor arrangement.
  - **Sound**: PipeWire default sinks/sources, per-device volume profiles, microphone noise gate.
  - **Network**: Wi-Fi network scanner, VPN toggles, DNS configuration.
  - **Panels & Dock**: Edge anchor selection (Left, Bottom, Right, Top), icon size, autohide timing, multi-monitor behavior.
  - **Gestures & Shortcuts**: 3-finger trackpad swipe mappings, global keyboard hotkeys.
- **Deep Full-Text Search**: In-memory index across all setting labels, sub-options, and descriptions, filtering matching sections and highlighting controls in real-time without hunting through menus.

## 3. `unbound-watchdog` (Force Quit & Unresponsive App Monitor)
- Works via the compositor's `wl_display.ping` heartbeat. If a client stops responding to `pong` for >3s, its window is desaturated and marked unresponsive.
- Pressing `Super+Escape` opens an unkillable, lightweight process viewer showing unresponsive apps, CPU/RAM stats, and instant `SIGTERM` / `SIGKILL` termination.

## 4. `unbound-monitor` (Activity Monitor & Profiler)
- Real-time CPU per-core utilization topology.
- Memory pressure gauge: Active, Wired, Cached, and zram/swap.
- GPU engine load and VRAM occupancy via DRM sysfs.
- Disk and Network I/O real-time streaming bandwidth.

## 5. `unbound-launcher` (Universal Application Launcher & Search Palette)
- **Global Invocation (`Super+Space`)**: Rendered on the `wlr-layer-shell` overlay layer with instant sub-16ms response time.
- **Search & Adaptive Ranking**: In-memory Tantivy / SQLite FTS5 index spanning installed desktop applications (`.desktop`), recent documents, and system preference panes. Adaptive frequency-recency ranking prioritizes frequently accessed items.
- **Math & Conversion Engine**: Inline evaluation of mathematical expressions and unit/currency conversions (pure Rust via `evalexpr`) with `Enter` or `Super+C` copying results directly to clipboard.
- **Quick Look Integration**: Arrowing over any file or media result and pressing `Space` displays an inline `unbound-ql` preview directly within the launcher card.

## 6. `unbound-screenshot` (Interactive Capture & Drag-to-App Workflow)
- **Workflow Keybindings**:
  - `Super+Shift+3`: Instant full-screen capture.
  - `Super+Shift+4`: Interactive crosshair with real-time dimension readout; pressing `Space` toggles window selection mode with automatic rounded-corner drop shadow and transparent background clipping.
  - `Super+Shift+5`: Floating control palette for regional/full screen recording, audio source selection, and timer delays.
- **Floating Thumbnail & Zero-Clutter Drag-to-App**:
  - Captured screenshots display an ephemeral thumbnail in the bottom-right corner for 4 seconds.
  - Clicking opens in-place annotation, cropping, and markup.
  - **Direct Drag**: Users can drag the thumbnail directly into chat applications, browser windows, or emails—transferring the image buffer immediately via Wayland `wl_data_device` without creating temporary files on disk.

## 7. Workspaces & Continuous Gesture Navigation
- **Workspace Architecture**: Linear horizontal workspaces with dynamic creation and per-workspace window tracking.
- **Continuous Gesture Kinematics**:
  - 3-finger horizontal trackpad swipe tracks finger displacement 1:1 using `zwp_pointer_gestures_v1`.
  - Reversing direction mid-swipe moves the workspace view backward seamlessly.
  - Upon finger lift, the instantaneous velocity vector feeds the compositor's spring physics solver ($\ddot{x} + 2\zeta\omega\dot{x} + \omega^2\Delta x = 0$) to complete the transition or settle back.
- **Workspace Overview**: 3-finger swipe up scales all workspace windows into a non-overlapping spatial overview with workspace switcher strip.
- **Keyboard Navigation**: `Super+1..9` jumps directly to workspace $N$; `Super+Ctrl+Left/Right` navigates sequentially.

## 8. Window Snapping & Tiling Zones
- **Edge & Corner Snapping**: Dragging a window to the left/right screen edge previews a 50% split-screen tile; dragging to any corner previews a 25% quarter-screen tile.
- **Keyboard Shortcuts**: `Super+Left/Right` splits active window to half screen, `Super+Up` maximizes, `Super+Down` restores or minimizes.
- **Layout Persistence**: Tiled windows maintain their proportional splits when display resolution or orientation changes.

## 9. `unbound-term` (GPU-Accelerated Terminal Emulator)
- Built in pure Rust on **`unbound-toolkit`** (Vello and Parley) with glyph caching.
- **Semantic Shortcut Cleanliness**: `Super+C` copies selected text and `Super+V` pastes from clipboard; `Ctrl+C` sends standard `SIGINT` to the running process without collision or mode switching.
- **Productivity Features**: Native split panes, tabs, scrollback search, clickable URL detection, and automatic theme synchronization with desktop light/dark appearance.
