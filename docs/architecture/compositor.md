# Compositor

`unbound-compositor` is a layered compositor, the architecture COSMIC and niri use: Smithay
handles the kernel and the Wayland protocols, and everything that decides how frames look and
when they reach the screen is Unbound's own.

## Layers

- **Kernel and protocol abstraction**: `smithay` handles DRM/KMS atomic modesetting, libinput
  devices, dma-buf import and export, and dispatch of the standard Wayland protocols.
- **Renderer**: Unbound's own Vulkan renderer on `ash` (`crates/platform/render`). The choice over
  `wgpu` and Smithay's GLES renderer, and the performance targets, are recorded in
  [`../decisions/0001-rendering.md`](../decisions/0001-rendering.md). Planned on top of it:
  dual-pass Kawase blur, rounded-corner clipping and transforms.
- **Motion**: a spring model ($\ddot{x} + 2\zeta\omega\dot{x} + \omega^2\Delta x = 0$) drives
  window snapping, minimizing to the Dock and moving between workspaces.

## Implemented

- **Renderer.** `crates/platform/render` is one Vulkan 1.3 device with push descriptors and
  dynamic rendering, implementing Smithay's `Renderer`, `Frame`, `ImportMemWl`, `ImportDmaWl`,
  `Offscreen` and `Bind<Dmabuf>`. Both outputs compose the same Wayland surface elements through
  Smithay: the headless output with `OutputDamageTracker` into a persistent offscreen image (read
  back only for tests), the DRM output with `DrmCompositor`. Each surface keeps a texture for its
  lifetime and an import uploads only the buffer's damaged rows; the damage tracker repaints only
  damaged regions and skips what an opaque window covers. The output is laid out in points and
  drawn in pixels at its scale.
- **Window shape and depth.** Every window that is not full screen is clipped to its
  `xdg_surface` window geometry with 12 pt rounded corners (antialiased in the fragment shader)
  and casts a soft shadow drawn analytically (a Gaussian-blurred rounded box): the key window's
  is larger and darker (blur 14 pt, 10 pt down, 42 %) than other windows' (7 pt, 4 pt, 22 %).
  The shadow is cut out under the window, so translucent windows do not show it through. All
  windows get this, including client-side decorated toolkits that draw their own shadow outside
  the geometry: clipping replaces it with ours, so every app looks alike. Full-screen windows
  have neither and keep direct scanout. A window's opaque region loses exactly the corner
  pixels its shape leaves uncovered, so damage below an opaque window is still skipped, and a
  shadow's element covers its blur reach, so moving or focusing a window damages its shadow.
  `ci/perf` damages a 64 × 64 patch inside the top window rather than at its corner, since the
  corner now lets what lies below show through by design.
- **Title bars for other toolkits.** A window that binds `xdg-decoration` gets a server-side
  title bar unless it asks for client-side (Qt, foot, SDL and Electron ask for or accept server
  side; GTK and our toolkit draw their own and never bind it). The bar is the toolkit's: the
  same 28 pt height, traffic lights, centred semibold title, colours of the light or dark
  palette (followed from the portal's `color-scheme`) and active, inactive and hover states,
  drawn by `unbound_graphics::TitleBar`, which `unbound-toolkit-platform` draws its own with.
  It is rasterised on the CPU at the output scale whenever its look changes and becomes part
  of the window: inside its rounded clip and shadow, in the opening, closing and overview
  animations. The window's geometry includes the bar, so placement, zoom and tiling size the
  app to what is left below it. Dragging the bar moves the window (with snapping), a double
  click zooms, the buttons close, minimise and zoom (a press that slides off does nothing, as
  in AppKit), and a 6 pt band outside the window resizes it with the matching cursor. A
  window never opens with its title bar above the top of the screen.
- **dma-buf.** `linux-dmabuf-v1` serves single-plane ARGB8888 and XRGB8888 buffers with the
  LINEAR modifier: version 3 headless, version 4 with feedback (main device and a scanout tranche
  of the primary plane's formats) on a DRM device, whose node accepted dma-bufs carry so that
  `DrmCompositor` can scan them out. A dma-buf is imported once as a Vulkan buffer and its
  damaged rows are copied into the surface's texture, so any stride works on drivers without
  `VK_EXT_image_drm_format_modifier`. Explicit sync (`linux-drm-syncobj-v1`) is built; its guest
  tests do not pass yet (`NOTES/backlog.md`).
- **DRM backend.** `unbound-compositor` opens the first card with a connected connector (or
  `--drm-device`) through a libseat session, takes the connector's preferred mode, and renders
  into GBM buffers that the renderer binds as dma-bufs (copied out from its own image, so
  lavapipe and `kms_swrast` work); `--headless` keeps the offscreen output for CI and the bench.
  A fullscreen opaque dma-buf window is put on the primary plane without composition. Tested on
  vkms in the `ci/vm` guest by reading the CRTC's framebuffer back.
- **Frame pacing.** Frames are paced by a vblank clock (virtual at 120 Hz in the headless
  backend; on DRM re-anchored to each page flip at the mode's refresh, advertised as the
  `wl_output` mode). A commit is latched into the frame whose deadline it arrives before: the
  vblank minus the longest time from deadline to finished render over the last 16 frames, minus
  1 ms. Frame callbacks and `wp_presentation` feedback (vblank time, refresh, sequence, `vsync`)
  are taken when the frame is latched and sent at its vblank (on DRM, at the page-flip event
  with its time and sequence); a superseded commit's feedback is discarded, and a render that
  overruns its vblank is presented at the next one.
- **Window motion.** Every window state change is animated by springs on x, y, width, height
  and opacity, sampled at the frame's vblank (`Headless::animate`, called by the scheduler
  before it latches a frame). Opening grows from 95 % and fades in (`standard`); closing keeps
  the last frame's textures and shrinks to 92 % while fading out (`quick`); minimising scales
  into the app's Dock icon, reported with `unbound_shell_v1.set_icon_rect`, or toward the
  Dock's centre (the bottom edge without a Dock), drawn above the Dock (`gentle`); restoring
  plays it back. Zoom and full screen (`standard`, `gentle`) start at the request and stretch
  the current buffer until the app commits its new size. Each operation only sets a new
  destination, so a second action mid-flight turns back from the current position and
  velocity. Input, focus and configure go to the destination at once; the input stack never
  contains a window that is only still animating. An animation that has never been sampled
  is drawn at its destination. Reduce Motion follows the Settings portal's
  `org.freedesktop.appearance` `reduced-motion` key (read once at session start, then on
  `SettingChanged`): windows and the snap preview keep their rectangles and fade with `fade`.
- **Snapping and tiling.** Dragging a window until the pointer touches the left or right screen
  edge previews the half, within 64 pt of a corner the quarter, and against the top of the
  usable area zoom; the translucent rounded preview grows from the pointer (`quick`) only after
  the pointer has rested 150 ms within 2 pt, so a fast drag past an edge never tiles. Releasing
  over a shown preview springs the window into it. `Ctrl+Super+Left/Right` tile halves,
  `Up/Down` on a half give its top or bottom quarter and on a quarter go back to the half,
  `Up` zooms a free window and `Down` returns a zoomed one. Tiles get xdg-shell's tiled states
  (for clients binding version 2) and no gaps. Dragging the edge two neighbouring tiles share
  resizes both; dragging a tiled window away gives back its earlier size under the pointer.
- **Drag and drop.** The compositor draws the drag icon a client passes to `start_drag` at the
  pointer, offset by the icon's attach offset, below the pointer. Holding Option (Alt) makes a
  drop a copy and Super a move, when source and destination offer it; otherwise the
  destination's preferred action wins. The choice is made when the destination sets its
  actions, so toolkits should set them again on motion for modifier changes to apply mid-drag.
  A drop nobody accepts slides the icon back to where the drag began with `standard` (with
  Reduce Motion it fades where it was dropped), from the last drawn frame of the icon, so it
  survives the client destroying the icon.
- **Gestures.** The compositor claims four-finger pinches (thumb and three fingers) and
  forwards every other swipe, pinch and hold through `zwp_pointer_gestures_v1`. A spread pushes
  every window aside to a 24 pt sliver at the nearer side edge, tracking the fingers 1:1 (a
  scale change of 0.6 is the whole way, with the 0.55 rubber band beyond); on release the
  progress projects with `Decay::NORMAL` from a least-squares velocity over the last 100 ms,
  rounds to shown or hidden and settles with `momentum` above 2 pages/s, `standard` below and
  `fade` with Reduce Motion. While the desktop shows, windows take no input and the keyboard
  leaves the app; pinching back or activating any window brings them back and returns the
  keyboard. A four-finger pinch from the normal state opens the launcher. The compositor also
  claims three-finger swipes: once the fingers have moved 16 units the axis locks, and a
  horizontal swipe switches workspaces, a swipe up opens the overview and a swipe down shows
  the focused app's windows (below). Two- and four-finger swipes go to apps.
- **Workspaces.** The output has one row of workspaces. A window opens on the active one
  (a window of another app opened while a full-screen workspace is active opens on that
  workspace's home). Workspaces are made on demand: moving a window past the last one adds
  one, and so does going full screen, which gives the window a workspace of its own right
  of its home, as on a Mac; leaving full screen takes the window home and the full-screen
  workspace goes away. An empty workspace goes away once the view has settled elsewhere,
  except the home of a full-screen workspace and the last ordinary one. Switching slides
  the whole row with `standard`: windows move with it while the wallpaper stays, and the
  keyboard goes to the frontmost window there. A three-finger horizontal swipe tracks the
  fingers 1:1 (350 units per workspace, rubber band past either end) and settles like the
  show-desktop pinch, with `momentum` above 2 workspaces/s; a new swipe during the settle
  takes over from where the row is. Keyboard (Super is the command key, Ctrl stays with
  terminals, so `Ctrl+Left/Right` is not taken): `Alt+Super+Left/Right` switch to the
  neighbour, `Alt+Super+1`…`9` to a workspace by position, and `Ctrl+Super+Shift+Left/Right`
  carry the focused window along (the window stays put on screen while the row slides
  under it; past the last workspace a new one is made). A window being dragged by its title
  bar comes along to whichever workspace the user switches to. A switch started while the
  row is still moving keeps its velocity. `unbound_app_v1.move_to_workspace`
  (v3) moves a window from its menu without following it: it slides off the screen and the
  keyboard goes to the next window. Focusing a window anywhere (the Dock's activate app or
  window, Super+Tab, an activation token) shows its workspace first. Shell clients learn the
  row through `unbound_shell_v1` v9: `workspaces(count, active)` and
  `window_workspace(identifier, position)`, sent on binding and on every change. With
  Reduce Motion a switch crossfades the two workspaces in place.
- **Overview.** `Alt+Super+Up`, the Mission Control key (`XF86LaunchA`, F3 on Apple
  keyboards, without Super) or a three-finger swipe up (300 units for the whole way, tracked
  1:1 and released like the other swipes) open it; the same key, `Escape`, a click on empty
  space or a swipe down close it. There is no hot corner: the research makes corners an
  opt-in setting, and no setting exists yet. Every window of the active workspace flies from
  where it is to a layout without overlap (rows by original height, left to right by
  original position, one common scale, never enlarged) under the workspace strip, and back
  when the overview closes, with `standard`; the drawing is the window's own live buffer
  scaled, never a copy. The strip slides down from the top: one live thumbnail per workspace
  (wallpaper and windows, scaled from the same textures), the active one ringed, and a "+"
  that adds a workspace. The selected window (the key window at first, then the one under
  the pointer or reached with the arrow keys) is ringed and shows its title below it, drawn
  with `unbound-graphics` once per title. A click on a window or `Return` focuses it, a click
  on a thumbnail switches there and closes the overview, and dragging a window by 4 pt
  lifts it under the pointer: dropped on a thumbnail it moves to that workspace, on the "+"
  to a new one, elsewhere it springs back. Typing filters by title or app id (the others dim
  to 25 % and cannot be selected), `Backspace` edits and the first `Escape` clears the
  filter. Keys never reach the app while the overview is open, except Super shortcuts, so
  `Alt+Super+Left/Right` and a horizontal swipe move between workspaces inside it, each
  workspace with its own layout. Empty workspaces are kept until the overview closes. App
  windows (App Exposé) show only one app's windows from every workspace without the strip:
  `Alt+Super+Down` or a three-finger swipe down for the focused app, and
  `unbound_shell_v1.show_windows(app_id)` (v9; an empty app id opens the plain overview) for
  the Dock. With Reduce Motion the windows stay where they are and fade out while the
  overview fades in.
- **Not responding.** Whenever the user interacts with a window, and for every app when Force
  Quit opens, the compositor pings the app through `xdg_wm_base.ping` (at most once a second).
  An app that has not answered after 3 s is not responding: its windows are veiled in a soft
  grey (24 %), the pointer over them is the theme's `wait` cursor, and shell clients get
  `unbound_shell_v1.app_responding(pid, app_id, 0)` so Force Quit can mark it. The answer, or
  the app's exit, lifts the veil and sends `responding 1`.
- **Springs.** `crates/libs/spring` is the closed-form solution for under-, critically and
  overdamped springs, parameterised by angular frequency $\omega$ and damping ratio $\zeta$. A
  trajectory depends only on elapsed time, never on frame rate, and retargeting keeps position
  and velocity continuous.
- **Session.** The compositor starts the shell clients and the clipboard daemon on socket pairs
  and restarts them (see [behaviour](behaviour.md)); `unbound-compositor --greeter` runs only
  the greeter ([session](session.md#display-manager-and-greeter)).

## Wayland protocol matrix

Priority: **P0** is needed for a daily-driver desktop, **P1** for full parity, **P2** later.
Status is what `crates/platform/compositor` serves on `main`.

| Protocol | Purpose | Priority | Status |
| :--- | :--- | :---: | :--- |
| `wl_compositor` / `wl_subcompositor` | Core surfaces and hierarchy | P0 | implemented |
| `wl_shm` | Shared-memory buffers | P0 | implemented |
| `wl_seat` (`pointer`, `keyboard`, `touch`) | Input | P0 | pointer and keyboard implemented, touch not |
| `linux-dmabuf-v1` | Zero-copy GPU buffer import | P0 | implemented (see above) |
| `linux-drm-syncobj-v1` | Explicit sync | P0 | built, guest tests failing |
| `xdg_wm_base` | Application windows and popups | P0 | implemented |
| `wlr-layer-shell-unstable-v1` | Dock, menu bar, notifications, overlays | P0 | implemented |
| `wp_fractional_scale_v1` / `wp_viewporter` | HiDPI and fractional scaling | P0 | implemented |
| `wp_presentation` | Presentation timing, VSync | P0 | implemented |
| `ext-session-lock-v1` | Screen locking | P0 | implemented |
| `wl_data_device_manager` | Clipboard and drag and drop | P0 | implemented |
| `zwp_primary_selection_v1` | Middle-click paste | P0 | implemented |
| `ext-data-control-v1` | Clipboard daemon access | P0 | implemented, trusted connection only |
| `ext-idle-notify-v1` / `idle-inhibit-v1` | Idle transitions, video playback | P0 | implemented |
| `xdg_activation_v1` | Focus-stealing prevention | P0 | implemented |
| `ext-foreign-toplevel-list-v1` | Window list for the Dock | P0 | implemented, shell clients only |
| `zwp_text_input_v3` / `input_method_v2` / `virtual_keyboard_v1` | Input methods (CJK) | P0 | implemented |
| `zxdg_decoration_manager_v1` | Decoration negotiation (server-side unless the client asks for client-side) | P1 | implemented |
| `wlr-output-management-v1` | Display arrangement and hotplug configuration | P1 | not implemented |
| `ext-image-capture-source-v1` / `ext-image-copy-capture-v1` | Screen capture for screenshots and the portal | P1 | implemented, shell clients only: output and toplevel sources (a window alone, title bar and rounded corners kept, shadow left out, transparent around it, from any workspace and while minimised), shm buffers; a session's first frame is rendered at once and later frames wait for the screen to change; a resized window announces its new size before failing the frame; window frames are withheld while the session is locked; cursor sessions are not implemented |
| `unbound_capture_v1` (own, `unbound-capture-v1.xml`) | Window frames on the current workspace for the screenshot tool's window mode, `show_options` on Super+Shift+5 | P1 | implemented, shell clients only |
| `wp_cursor_shape_v1` | Cursor shapes from the theme | P1 | implemented |
| `zwp_pointer_constraints_v1` / `relative_pointer` | Pointer lock for games | P1 | not implemented |
| `zwp_pointer_gestures_v1` | Trackpad gestures | P1 | implemented, v3: swipes, pinches and holds the compositor does not claim go to the client under the pointer |
| `zwp_keyboard_shortcuts_inhibit_v1` | Keyboard passthrough for VMs and remote desktop | P2 | not implemented |
| `unbound_shell_v1` (own) | Launcher, screenshots, active app, volume keys, Control Center, Dock icon rectangles, apps that stopped responding; for the Dock (v7): hide, quit and force quit an app, activate one window by its foreign-toplevel identifier, minimised windows, attention requests, rounded blur for shell surfaces, Super+F3 and Super+F2 keyboard focus for the Dock and menu bar; v9: workspaces, the workspace of each window and `show_windows` for the overview and App Exposé | P0 | implemented, shell clients only |
| `unbound_app_v1` (own, in `unbound-shell-v1.xml`) | App-level requests: `quit_requested` on Super+Q instead of closing every window; v2 `place_window` (halves, quarters, restore) for the window menu; v3 `move_to_workspace` (previous, next) for the window menu | P0 | implemented, every client |
| `ext-background-effect-v1` | Blur behind translucent menus, sidebars, Dock and menu bar | P1 | implemented: dual-filter Kawase blur (3 passes) clipped to the window's rounded shape, below the window's own shadow, redrawn when anything beneath it changes |
