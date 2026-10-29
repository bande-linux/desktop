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
  is drawn at its destination. Reduce Motion (`Headless::set_reduce_motion`; not yet read
  from the portal's `reduced-motion`) keeps rectangles in place and fades with `fade`.
- **Snapping and tiling.** Dragging a window until the pointer touches the left or right screen
  edge previews the half, within 64 pt of a corner the quarter, and against the top of the
  usable area zoom; the translucent rounded preview grows from the pointer (`quick`) only after
  the pointer has rested 150 ms within 2 pt, so a fast drag past an edge never tiles. Releasing
  over a shown preview springs the window into it. `Ctrl+Super+Left/Right` tile halves,
  `Up/Down` on a half give its top or bottom quarter and on a quarter go back to the half,
  `Up` zooms a free window and `Down` returns a zoomed one. Tiles get xdg-shell's tiled states
  (for clients binding version 2) and no gaps. Dragging the edge two neighbouring tiles share
  resizes both; dragging a tiled window away gives back its earlier size under the pointer.
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
| `zxdg_decoration_manager_v1` | Decoration negotiation (answers client-side) | P1 | implemented |
| `wlr-output-management-v1` | Display arrangement and hotplug configuration | P1 | not implemented |
| `ext-image-capture-source-v1` / `ext-image-copy-capture-v1` | Screen capture for screenshots and the portal | P1 | implemented (output sources) |
| `wp_cursor_shape_v1` | Cursor shapes from the theme | P1 | implemented |
| `zwp_pointer_constraints_v1` / `relative_pointer` | Pointer lock for games | P1 | not implemented |
| `zwp_pointer_gestures_v1` | Trackpad gestures | P1 | not implemented |
| `zwp_keyboard_shortcuts_inhibit_v1` | Keyboard passthrough for VMs and remote desktop | P2 | not implemented |
| `unbound_shell_v1` (own) | Launcher, screenshots, active app, volume keys, Control Center, Dock icon rectangles, apps that stopped responding | P0 | implemented, shell clients only |
| `unbound_app_v1` (own, in `unbound-shell-v1.xml`) | App-level requests: `quit_requested` on Super+Q instead of closing every window | P0 | implemented, every client |
| `ext-background-effect-v1` | Blur behind translucent menus, sidebars, Dock and menu bar | P1 | implemented: dual-filter Kawase blur (3 passes) clipped to the window's rounded shape, below the window's own shadow, redrawn when anything beneath it changes |
