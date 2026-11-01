# Compositor

`bande-compositor` is a layered compositor, the architecture COSMIC and niri use: Smithay
handles the kernel and the Wayland protocols, and everything that decides how frames look and
when they reach the screen is Bande's own.

## Layers

- **Kernel and protocol abstraction**: `smithay` handles DRM/KMS atomic modesetting, libinput
  devices, dma-buf import and export, and dispatch of the standard Wayland protocols.
- **Renderer**: Bande's own Vulkan renderer on `ash` (`crates/platform/render`). The choice over
  `wgpu` and Smithay's GLES renderer, and the performance targets, are recorded in
  [`../decisions/0001-rendering.md`](../decisions/0001-rendering.md). Planned on top of it:
  dual-pass Kawase blur, rounded-corner clipping and transforms. When the Vulkan device is a
  CPU (lavapipe in VMs and on machines without a usable GPU) the renderer reduces effects: a
  blur region becomes a flat scrim of the sheet colour at 72 % that does not depend on what
  lies beneath it, so moving something under the bar no longer redraws the bar.
- **Motion**: a spring model ($\ddot{x} + 2\zeta\omega\dot{x} + \omega^2\Delta x = 0$) drives
  window snapping, minimizing to the taskbar and moving between workspaces.

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
  side; our toolkit draws its own and never binds it). GTK 3 and GTK 4 speak only KDE's
  `org_kde_kwin_server_decoration`, whose manager announces server-side as the default: GTK
  windows without a header bar ask for server-side and get our bar, windows with a header bar
  (libadwaita, `gtk_window_set_titlebar`) ask for client-side and keep their own, and a later
  switch is honoured. A mode requested before the surface becomes a toplevel is kept and
  applied when it does. The bar is `bande_graphics::TitleBar`, which `bande-toolkit-platform`
  draws its own with: 40 pt high, a pill on the left with close, minimise and zoom, a flush-left
  Manrope title, colours of the light or dark palette (followed from the portal's
  `color-scheme`) and active, inactive and hover states.
  It is rasterised on the CPU at its display's scale whenever its look changes (and stretched
  to fit while it is drawn on another display mid-drag or in a mirror) and becomes part
  of the window: inside its rounded clip and shadow, in the opening, closing and overview
  animations. The window's geometry includes the bar, so placement, zoom and tiling size the
  app to what is left below it. Dragging the bar moves the window (with snapping), a double
  click zooms, the buttons close, minimise and zoom (a press that slides off does nothing, as
  in AppKit), and a band from 8 pt outside to 3 pt inside the rounded outline resizes it with the
  matching cursor (`resize_zone` in `bande-graphics`, shared with the toolkit). A floating
  client-side window gets the outside 8 pt too wherever its own input region leaves the
  point to the compositor. A
  window never opens with its title bar above the top of the screen.
- **dma-buf.** `linux-dmabuf-v1` serves single-plane ARGB8888 and XRGB8888 buffers with the
  LINEAR modifier: version 3 headless, version 4 with feedback (main device and a scanout tranche
  of the primary plane's formats) on a DRM device, whose node accepted dma-bufs carry so that
  `DrmCompositor` can scan them out. A dma-buf is imported once as a Vulkan buffer and its
  damaged rows are copied into the surface's texture, so any stride works on drivers without
  `VK_EXT_image_drm_format_modifier`. Explicit sync (`linux-drm-syncobj-v1`) is built; its guest
  tests do not pass yet.
- **DRM backend.** `bande-compositor` opens the first card with a connected connector (or
  `--drm-device`) through a libseat session and drives every connected connector on its own
  CRTC, each with its own `DrmCompositor`, frame clock and scheduler; it renders into GBM
  buffers that the renderer binds as dma-bufs (copied out from its own image, so lavapipe and
  `kms_swrast` work); `--headless` keeps the offscreen output for CI and the bench. udev change
  events rescan the connectors, so plugging a display in or out takes effect without restarting
  the session. A display is named by its EDID (maker from the PNP id, model and serial from the
  descriptors, falling back to the product code and serial number; two displays with the same
  name are told apart by their connector). A fullscreen opaque dma-buf window is put on the
  primary plane without composition. Tested on vkms in the `ci/vm` guest by reading the CRTC's
  framebuffer back.
- **Displays.** Every output lives in one logical coordinate space in points. Its mode, scale
  (fractional too), rotation and place come from `display` and `arrangement` in `config.kdl`
  (followed live through `org.bande.Config`): the arrangement remembered for exactly the
  connected set, else the largest remembered subset with the rest placed to the right; a
  display nobody configured gets its preferred mode at its highest refresh rate, the automatic
  scale for its pixel density and a place to the right of the others, top-aligned with the main
  display. The main display sits at the origin; it is the remembered one, else the one that was
  main before. Each output keeps its own layer surfaces (`wlr-layer-shell` surfaces without an
  output go to the main display, except the launcher, the screenshot palette, Control Center
  and confirmations, which open on the display with the pointer), its own lock surface and its
  own row of workspaces. A window belongs to one display (through its workspace) and is drawn
  only there, except while it is dragged across an edge; it is told the display through
  `wl_surface.enter`/`leave` and its scale through `wp_fractional_scale_v1`, so it redraws
  sharp when it moves to a display of another scale. The pointer moves across the displays at
  their own scales and never leaves them (it stops at an edge without a neighbour); a new
  window opens on the display with the pointer, cascading only from its app's windows shown on
  that display, maximising, full screen, tiling and snapping
  use the window's display, and dragging a window until the pointer crosses onto another
  display moves it there. `Shift+Super+Left/Right/Up/Down` move the focused window to the
  neighbouring display in that direction keeping its relative place (shrunk only when it does
  not fit), which it reaches with the opening spring; a window with our title bar keeps it
  across displays and the app is sized to what is left below it. When a display's scale or
  resolution changes the pointer keeps its relative place on it. When a display goes away its shown
  workspace's windows join the main display's shown workspace and its other workspaces join the
  main display's row, each window keeping its relative place; when it returns, those windows
  and workspaces go back to it, at the place they had. `wlr-output-management-v1` lists the
  displays (the head description is the display's name) and applies configurations from tools
  such as `wlr-randr`; Settings reads it. A display can mirror another (`mirror=` in the
  arrangement): it keeps no windows or workspaces of its own, the pointer does not enter it, and
  it shows its source drawn again at the scale that fits it, centred. A rotated display is laid
  out upright in points and the renderer turns every quad, clip and shadow into the framebuffer.
  The desktop surface and the menu bar are drawn on every
  display: the compositor tells the shell the displays that show their own picture, the main one
  first (`bande_shell_v1.displays`, since 15, on binding and whenever a display comes, goes,
  starts or stops mirroring or the main display changes), and the shell opens a desktop and a taskbar
  on each of them in its one process; on every display but the main one the desktop shows only
  the wallpaper and no icons, and the separate layout's apps strip, the menu search and the notifications stay on the main
  display. The lock
  screen asks for the password on the main display; while locked, every other display shows only
  its wallpaper.
- **Settings ▸ Displays.** An arrangement of the displays (shown when there are two or more)
  lets the user drag a display's rectangle; on release it snaps edge to edge beside the nearest
  display, aligning tops, bottoms or centres within 48 pt, and never onto another display; with
  the keyboard, Space picks a display and the arrows put it on that side of the main display.
  The main display carries a white bar for its menu bar. Each display then offers Use as (Main
  display, Extended, Mirror), Resolution (up to four sizes of its native shape), Size of text
  and controls, Refresh rate, Rotation and Variable refresh rate. Every change is written to
  `config.kdl` and applied live; a new resolution or rotation shows "Keep these display
  settings?" with Keep and Revert and comes back by itself after 15 s, as macOS does.

- **Frame pacing.** Frames are paced by a vblank clock (virtual at 120 Hz in the headless
  backend; on DRM re-anchored to each page flip at the mode's refresh, advertised as the
  `wl_output` mode). A commit is latched into the frame whose deadline it arrives before: the
  vblank minus the longest time from deadline to finished render over the last 16 frames
  (capped at one refresh, so a vblank is always reachable), minus 1 ms. The longest rather than
  a percentile is deliberate: heavier content, such as the overview opening, then misses at
  most one vblank instead of several while a percentile catches up, and on a loaded machine
  wake-up jitter is what makes frames late. When a wake-up is so late that the typical render
  time (a moving average) no longer fits before the planned vblank, the frame is drawn at once
  for the next vblank it can reach, with animations sampled for that vblank, so it is not shown
  one refresh stale and the scheduler does not sleep (and risk waking late) again. Frame callbacks and `wp_presentation` feedback (vblank time, refresh, sequence, `vsync`)
  are taken when the frame is latched and sent at its vblank (on DRM, at the page-flip event
  with its time and sequence); a superseded commit's feedback is discarded, and a render that
  overruns its vblank is presented at the next one.
- **Window motion.** Every window state change is animated by springs on x, y, width, height
  and opacity, sampled at the frame's vblank (`Compositor::animate_on`, called by each output's
  scheduler before it latches a frame). Animations are closed-form springs, so every display
  samples its own vblank, even one earlier than another display's last frame, and displays
  of different rates never repeat a frame. An animation stops once its remaining motion is
  invisible: within 0.25 physical pixel (1/512 for opacity) and slower than that per 20 ms.
  Opening grows from 96 % and fades in (`standard`); closing keeps the last frame's textures
  and shrinks to 98 % while fading out (`quick`); minimising scales
  into the app's taskbar icon, reported with `bande_shell_v1.set_icon_rect`, or toward the
  taskbar's centre (the bottom edge without a taskbar), drawn above the taskbar (`gentle`); restoring
  plays it back. Zoom and full screen (`standard`) start at the request and stretch
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
  over a shown preview springs the window into it. `Super+Left/Right` tile halves,
  `Up/Down` on a half give its top or bottom quarter and on a quarter go back to the half,
  `Up` zooms a free window and `Down` returns a zoomed one. Tiles get xdg-shell's tiled states
  (for clients binding version 2) and no gaps. Dragging the edge two neighbouring tiles share
  resizes both; dragging a tiled window away gives back its earlier size under the pointer.
- **Drag and drop.** The compositor draws the drag icon a client passes to `start_drag` at the
  pointer, offset by the icon's attach offset, below the pointer. Holding Ctrl makes a
  drop a copy and Shift a move (as in GTK and KDE), when source and destination offer it; otherwise the
  destination's preferred action wins. The choice is made when the destination sets its
  actions; pressing or releasing Ctrl or Shift mid-drag sends the destination a motion at the
  same place, so a destination that sets its actions again on motion (ours and GTK do) gets the
  new action without the pointer moving. The icon is drawn on every display it overlaps, and
  the pointer shows what a drop would do: `copy` for a copy, `move` (the arrow in most themes)
  for a move, `no-drop` where nothing accepts. A drag whose source dies ends at once; when the
  surface under the drag dies the drag moves on to what is now under the pointer.
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
- **Workspaces.** Each display has its own row of workspaces (macOS's "Displays have separate Spaces"); swipes and switching shortcuts act on the display with the pointer, the overview opens on every display with that display's windows and strip (a window dropped on another display moves there), and shell clients learn the row of the display with the key window. A window opens on the active one
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
  takes over from where the row is. Keyboard (Super is the system key and Ctrl belongs to
  apps, so `Ctrl+Left/Right` alone is not taken): `Ctrl+Super+Left/Right` switch to the
  neighbour (as in KDE and Windows), `Ctrl+Super+1`…`9` to a workspace by position, and `Ctrl+Shift+Super+Left/Right`
  carry the focused window along (the window stays put on screen while the row slides
  under it; past the last workspace a new one is made). A window being dragged by its title
  bar comes along to whichever workspace the user switches to. A switch started while the
  row is still moving keeps its velocity. `bande_app_v1.move_to_workspace`
  (v3) moves a window from its menu without following it: it slides off the screen and the
  keyboard goes to the next window. Focusing a window anywhere (the taskbar's activate app or
  window, Super+Tab, an activation token) shows its workspace first. Shell clients learn the
  row through `bande_shell_v1` v9: `workspaces(count, active)`, sent on binding and on
  every change. With Reduce Motion a switch crossfades the two workspaces in place.
- **Overview.** `Super+W` (as in KDE), the Mission Control key (`XF86LaunchA`, F3 on Apple
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
  with `bande-graphics` once per title. A click on a window or `Return` focuses it, a click
  on a thumbnail switches there and closes the overview, and dragging a window by 4 pt
  lifts it under the pointer: dropped on a thumbnail it moves to that workspace, on the "+"
  to a new one, elsewhere it springs back. Typing filters by title or app id (the others dim
  to 25 % and cannot be selected), `Backspace` edits and the first `Escape` clears the
  filter. Keys never reach the app while the overview is open, except Super shortcuts, so
  `Ctrl+Super+Left/Right` and a horizontal swipe move between workspaces inside it, each
  workspace with its own layout. Empty workspaces are kept until the overview closes. App
  windows (App Exposé) show only one app's windows from every workspace without the strip:
  `Alt+Super+Down` or a three-finger swipe down for the focused app, and
  `bande_shell_v1.show_windows(app_id)` (v9; an empty app id opens the plain overview) for
  the taskbar. With Reduce Motion the windows stay where they are and fade out while the
  overview fades in.
- **Switcher, window memory and the other half.** `switcher.rs` holds the Super+Tab and
  Super+` session (behaviour.md has the rules): a dwell of 150 ms counted in vblank time, then
  a card laid out once per change (a row of tiles, shrunk uniformly to fit the display) whose
  frame and selection well are `FillElement`s driven by `Motion` on the `quick` spring, the
  icons and name painted on the CPU with `bande-graphics` into memory buffers at the output
  scale, and window thumbnails drawn from the windows' own textures as in the overview. App
  names and icons come from desktop entries through `bande-xdg`, cached, and reread at most
  every 30 s while an app is unknown. `access.rs` publishes the card through AccessKit (a
  window holding a list box whose selected option has the focus) only while it shows.
  `recall.rs` keeps the window memory file, `assist.rs` the free-half panel, which reuses the
  overview's `arrange` and the switcher's thumbnails.
- **Not responding.** Whenever the user interacts with a window, and for every app when Force
  Quit opens, the compositor pings the app through `xdg_wm_base.ping` (at most once a second).
  An app that has not answered after 3 s is not responding: its windows are veiled in a soft
  grey (24 %), the pointer over them is the theme's `wait` cursor, and shell clients get
  `bande_shell_v1.app_responding(pid, app_id, 0)` so Force Quit can mark it. The answer, or
  the app's exit, lifts the veil and sends `responding 1`.
- **Springs.** `crates/libs/spring` is the closed-form solution for under-, critically and
  overdamped springs, parameterised by angular frequency $\omega$ and damping ratio $\zeta$. A
  trajectory depends only on elapsed time, never on frame rate, and retargeting keeps position
  and velocity continuous.
- **Session.** The compositor starts the shell clients and the clipboard daemon on socket pairs
  and restarts them (see [behaviour](behaviour.md)); `bande-compositor --greeter` runs only
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
| `wlr-layer-shell-unstable-v1` | Taskbar, menu bar, notifications, overlays | P0 | implemented |
| `wp_fractional_scale_v1` / `wp_viewporter` | HiDPI and fractional scaling | P0 | implemented |
| `wp_presentation` | Presentation timing, VSync | P0 | implemented |
| `ext-session-lock-v1` | Screen locking | P0 | implemented |
| `wl_data_device_manager` | Clipboard and drag and drop | P0 | implemented |
| `zwp_primary_selection_v1` | Middle-click paste | P0 | implemented |
| `ext-data-control-v1` | Clipboard daemon access | P0 | implemented, trusted connection only |
| `ext-idle-notify-v1` / `idle-inhibit-v1` | Idle transitions, video playback | P0 | implemented |
| `xdg_activation_v1` | Focus-stealing prevention | P0 | implemented |
| `ext-foreign-toplevel-list-v1` | Window list for the taskbar | P0 | implemented, shell clients only |
| `zwp_text_input_v3` / `input_method_v2` / `virtual_keyboard_v1` | Input methods (CJK) | P0 | implemented |
| `zxdg_decoration_manager_v1` | Decoration negotiation (server-side unless the client asks for client-side) | P1 | implemented |
| `org_kde_kwin_server_decoration_manager` | Decoration negotiation for GTK 3 and GTK 4 (default server-side, client-side for header-bar windows) | P1 | implemented |
| `wlr-output-management-v1` | Display arrangement and hotplug configuration | P1 | implemented (v4; enabled heads only, no custom modes) |
| `ext-image-capture-source-v1` / `ext-image-copy-capture-v1` | Screen capture for screenshots and the portal | P1 | implemented, shell clients only: output and toplevel sources (a window alone, title bar and rounded corners kept, shadow left out, transparent around it, from any workspace and while minimised), shm buffers; a session's first frame is rendered at once and later frames wait for the screen to change; a resized window announces its new size before failing the frame; window frames are withheld while the session is locked; cursor sessions are not implemented |
| `bande_capture_v1` (own, `bande-capture-v1.xml`) | Window frames on the current workspace for the screenshot tool's window mode, `show_options` on Print | P1 | implemented, shell clients only |
| `wp_cursor_shape_v1` | Cursor shapes from the theme | P1 | implemented |
| `zwp_pointer_constraints_v1` / `relative_pointer` | Pointer lock for games | P1 | not implemented |
| `zwp_pointer_gestures_v1` | Trackpad gestures | P1 | implemented, v3: swipes, pinches and holds the compositor does not claim go to the client under the pointer |
| `zwp_keyboard_shortcuts_inhibit_v1` | Keyboard passthrough for VMs and remote desktop | P2 | not implemented |
| `bande_shell_v1` (own) | Launcher, screenshots, active app, volume keys, Control Center, taskbar icon rectangles, apps that stopped responding; for the taskbar (v7): hide, quit and force quit an app, activate one window by its foreign-toplevel identifier, minimised windows, attention requests, rounded blur for shell surfaces, Super+F3 and Super+F2 keyboard focus for the taskbar and menu bar; v9: workspaces, the workspace of each window and `show_windows` for the overview and App Exposé | P0 | implemented, shell clients only |
| `bande_app_v1` (own, in `bande-shell-v1.xml`) | App-level requests: `quit_requested` on the bar's or the taskbar's Quit instead of closing every window; v2 `place_window` (halves, quarters, restore) for the window menu; v3 `move_to_workspace` (previous, next) for the window menu | P0 | implemented, every client |
| `ext-background-effect-v1` | Blur behind the launcher's scrim (the overview's scrim and blur are the compositor's own) | P1 | implemented: dual-filter Kawase blur (3 passes) clipped to the window's rounded shape, below the window's own shadow, redrawn when anything beneath it changes; a flat scrim on a CPU renderer |
