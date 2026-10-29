# Backlog

Ordered milestones derived from `docs/ARCHITECTURE.md` (phases and protocol matrix) and
`docs/RENDERING.md`. Each milestone lives on `desktop/milestone-N`, stacked on the one
before, and is done when its definition of done holds on the self-hosted runners
(`ci/lint`, `ci/lavapipe-test`, `ci/deb`, and the jobs it adds) without real GPU or DRM
hardware. Things noticed in passing go to the end of this file, not into a diff.

## Toward the daily driver (75%)

Target: plan phases 1-6 done plus enough protocols for Firefox on Wayland. Pure Wayland:
XWayland is deferred, to revisit only if a needed app has no Wayland path (macOS
precedent: XQuartz is a separate, optional install). Progress is tracked per milestone
below. Estimate at the head of `desktop/milestone-19`: about 68% of the way to the target,
counting the six phases and Firefox equally: phase 1 done; phase 2 about 90% (explicit sync
left); phase 3 about 85% (backlight dimming, Do Not Disturb persistence);
phase 4 about 35% (panel and launcher, no menu bar, greeter, AT-SPI or input methods);
phase 5 about 40% (portal and power without their UIs, no audio, network or Bluetooth);
phase 6 about 45% (launcher, terminal, full-screen and area screenshots; no files or settings); Firefox
runs natively on Wayland in the guest.

## Done

- [x] **M1** Headless Smithay compositor, wgpu offscreen render on lavapipe, golden-image
  test, arm64 `.deb`.
- [x] **M2** `unbound-toolkit` (tokens, fills, text), `unbound-config` (KDL, atomic
  apply), `unbound-spring`.
- [x] **M3** `org.unbound.Config` on D-Bus with a file watcher, `unbound-notify`,
  clipboard protocols.
- [x] **M4** Compositor-started `unbound-clipboard` on a private socket, macOS focus
  rules, packaging, NVIDIA removed from the plan.

## Next

- [x] **M5 Rendering decision and performance harness** (`desktop/milestone-5`)
  `docs/RENDERING.md`; `crates/unbound-bench` measuring the compositor's typical and
  full-repaint frame times at 1920×1080, start time to listening socket and idle RSS;
  `ci/perf` runs it on every push, uploads `perf.json` and writes a summary table.
  *Done when* `ci/perf` is green and reports every metric.
  Baseline with the wgpu renderer on the runners: typical frame p50 11.4 ms / p99
  39.1 ms, full repaint p50 12.4 ms / p99 35.5 ms, start 2214 ms, idle RSS 93.6 MiB.
  Budgets are reported, not enforced, until M6.
- [x] **M6 `unbound-render`: Vulkan (`ash`) compositor renderer** (`desktop/milestone-6`)
  Persistent per-surface textures, `wl_shm` uploads limited to buffer damage,
  damage-limited repaint of the output, readback only on request; the compositor drops
  `wgpu`. *Done when* the golden test passes on lavapipe, tests show that repainted
  pixels equal the damaged area and uploaded bytes equal the damaged rows, and `ci/perf`
  enforces the lavapipe budgets of `docs/RENDERING.md`.
  Result on the runners: typical frame p50 0.28 ms / p99 0.51 ms (was 11.4 / 39.1),
  full repaint p50 4.6 ms / p99 11.4 ms, start p50 30 ms (was 2214), idle RSS 89 MiB.
  Opaque windows also hide what lies below them from shading.
- [x] **M7 `linux-dmabuf-v1` with Vulkan import** (`desktop/milestone-7`)
  Single-plane LINEAR ARGB/XRGB8888 buffers imported through
  `VK_EXT_external_memory_dma_buf`; version 3 global while there is no DRM device
  (feedback needs one). *Done when* a dmabuf window renders to the golden image on
  lavapipe and invalid buffers fail with the protocol's errors.
  Runners have no dma-buf exporter (no `/dev/udmabuf`, no DRM device), so tests back
  the buffers with a file that lavapipe imports through the same dma-buf path; real
  PRIME dma-bufs are exercised once the KVM rig (M9) exists.
- [x] **M8 Frame scheduling and `wp_presentation_time`** (`desktop/milestone-8`)
  Virtual 120 Hz vblank clock in the headless backend, composing at the deadline, frame
  callbacks and presentation feedback after presentation. *Done when* tests show a
  commit before the deadline is presented at the next vblank with the feedback's
  refresh interval and sequence, and `ci/perf` reports commit-to-present frames.
  Scheduling tests run on a virtual clock. On the runners' real clock: commit-to-present
  p50 11.4 ms / p99 16.3 ms, 4 of 120 frames after their promised vblank, reported
  only (see `docs/RENDERING.md`).
- [x] **M9 KVM test rig (`ci/vm`)** (`desktop/milestone-9`)
  Guest kernel built with `CONFIG_DRM_VKMS` and virtio-gpu, Debian 13 arm64 root image,
  QEMU with `-machine virt,its=off`; the job boots the guest and runs test binaries in
  it. *Done when* `ci/vm` boots and a guest test opens `/dev/dri/card0`.
  `ci/vm/kernel.sh` builds Linux 6.12.114 (Debian 13's series) from arm64 defconfig without
  modules or SoC platforms plus `kernel.config` (vkms, virtio-gpu, 9p, udmabuf, sw_sync,
  uinput); `rootfs.sh` exports a `debian:13-slim` image through the runners' Docker engine
  into an ext4 image. Both are cached with `actions/cache` keyed on their recipes (kernel
  build about 6 min, cached boot about 1 s). The guest runs `cargo test` executables of
  every `guest` test target over 9p with `--ignored`; tests check card0, vkms's CRTC,
  virtual and writeback connectors and primary, cursor and overlay planes, and virtio-gpu.
  `ci/perf` now gates only exact counts and idle RSS; wall-clock budgets are reported with
  the host load, since the shared host moved unchanged code's p50 tenfold.
- [x] **M10 DRM/KMS backend** (`desktop/milestone-10`)
  libseat session, atomic modesetting through Smithay's `DrmCompositor`,
  `unbound-render` implementing Smithay's renderer traits, direct scanout and overlay
  planes. *Done when* the compositor drives VKMS in the guest, a fullscreen opaque
  window is scanned out directly, and the writeback/screendump matches the golden.
  `unbound-render` now implements `Renderer`, `Frame`, `ImportMemWl`, `ImportDmaWl`,
  `Offscreen` and `Bind<Dmabuf>`; both outputs compose Smithay surface elements (headless
  through `OutputDamageTracker`, DRM through `DrmCompositor`), so the M6 exact metrics now
  cover the shipping path (perf unchanged: typical p50 0.28 ms). The DRM output renders
  into dumb buffers written through a CPU mapping (lavapipe cannot import GBM's read-only
  exports), linux-dmabuf v4 feedback names the card, and client dma-bufs are imported for
  scanout through GBM. Guest tests read the CRTC's framebuffer back through PRIME: a
  composed ARGB window matches the scene, and a fullscreen XRGB udmabuf window is put on
  the primary plane and matches it. `unbound-compositor` defaults to DRM; `--headless`
  keeps the offscreen output for CI and the bench.
- [x] **M11 First boot on the M2 Mac mini (hardware smoke test)**
  The compositor on the real M2 mini through the `hil` service (unbound-linux/hil; the
  host is the user's laptop, the mini on USB-C). The mini exposes the bootloader's
  framebuffer through `simpledrm` (no DCP, no GPU), so composition is lavapipe on the
  CPU, and `simpledrm` has one primary plane and no overlays, so `DrmCompositor` falls
  back to composing everything; the frame clock follows page-flip completion. The M9
  VM stays the gate on every push; this runs on demand and on milestone tips.
  *Done when* the mini boots, `unbound-compositor` takes the display through libseat,
  a test client maps a window, and a screendump or framebuffer readback matches the
  golden image within the SSIM bound.
  *Depends on* kernel work done by the user (not in this repo, nor the `hil` repo):
  K1 boot to userspace, K2 `simpledrm` on the boot framebuffer; and on M10.
  *Result (2026-10-28, `desktop/milestone-11`)*: with K1 and K2 done, `ci/hil/initramfs.sh`
  builds a 38 MB initramfs (guest test binary, seatd, lavapipe's library closure; Mesa's
  GBM backend left out, which the compositor now tolerates). Booted through
  `hil-host/boot-kis.sh` under the mini lock, `scanout::a_composed_window_is_scanned_out`
  passed on simpledrm at 1920×1080: libseat took the card, lavapipe composed the window
  and the framebuffer read back through PRIME matched the scene. The screendump is at
  `/Volumes/Unbound/hil/hw2/m11/mini-composed-window.png`. The upload runs at about
  60 KB/s, so the run takes about 12 minutes. Not automated in CI; input on the mini (K3)
  needs a person at the keyboard.
- [x] **M12 Keyboard and pointer input** (`desktop/milestone-12`)
  libinput through the libseat session, xkbcommon keymaps, `wl_keyboard`/`wl_pointer`
  with focus-follows-click and the macOS focus rules of section 1.7, cursor drawn by the
  compositor, and the `Super`-based shortcut layer of section 1.5. *Done when* guest
  tests inject events through `uinput` and clients receive them with the right focus,
  and on the mini once K3 (USB input in the kernel) lands.
  Headless tests drive the seat directly (click-to-focus and raise, keys to the focused
  client only, surface-local pointer enter, empty-desktop click, Super+W/Q/Tab, the arrow
  drawn once the pointer moves); the guest test creates a uinput keyboard-and-mouse,
  which libinput finds through udev, and checks that a click moves keyboard focus to the
  window under the pointer and that its keys and buttons reach only that client. Scroll
  axes, keyboard repeat settings, a hardware cursor plane and the mini (K3) are not done.
- [x] **M13 Shell protocols** (`desktop/milestone-13`) `wlr-layer-shell`,
  `xdg_activation_v1`, `wp_viewporter`, `wp_fractional_scale_v1`. *Done when* a layer
  surface reserves its exclusive zone and toplevels are placed outside it.
  Layers come from Smithay's layer map, drawn background, bottom, windows, top, overlay;
  new windows are centered in the area outside every exclusive zone (macOS's visible
  frame). A token is valid only if the focused app requested it (macOS cooperative
  activation) and activates its window now or when it maps; clicks on layer surfaces keep
  the key window (as the Dock does). Viewport destinations set the window size; surfaces
  are told scale 1.0. Not done: keyboard focus for layer surfaces with keyboard
  interactivity, layer popups, re-placing windows when a zone appears later. The
  first-window focus stand-in stays until the launcher (M14) passes activation tokens,
  since without it apps started from a terminal would open unfocused.
- [ ] **M14 Minimal shell: first usable desktop**
  `unbound-panel` (left edge by default, from `config.kdl`) with app buttons and a
  clock, `unbound-launcher` on `Super+Space` listing `.desktop` entries, and a minimal
  `unbound-term` (PTY, scrollback, `Super+C`/`Super+V`), all on the current toolkit.
  *Done when* golden tests cover each surface, a guest session test launches the
  terminal from the launcher and types into it, and the same works on the mini.
  Built on the Vello toolkit (`desktop/milestone-14`, on `desktop/integration`): toolkit
  windows as layer surfaces with seat input (xkb, repeat), clipboard, activation and images;
  `unbound-apps` (desktop entries, theme icons, launching); compositor shell protocol,
  window list, exclusive-layer keyboard focus, layer frame callbacks, wheel scrolling; the
  three apps with goldens; `desktop::the_launcher_opens_a_terminal_that_takes_typing` in
  the guest. Not done: the mini (K3 input), live `PanelEdge`/appearance changes over
  `org.unbound.Config` (read at start), kept apps from `config.kdl`, hover names on the
  panel, locale time formats, terminal mouse reporting, AT-SPI publication.
- [x] **M15 Window management** (`desktop/milestone-15`, CI green in run 40781875368)
  xdg move and resize grabs from the client's button press, zoom (maximize) into the
  area outside exclusive zones and back, full screen above top layers, minimize, the
  `Activated` state on the key window only, xdg popups placed by their positioner with
  grabs that a click outside dismisses without delivering it, `Super+M` and
  `Ctrl+Super+F`. macOS precedents: windows stay below the menu bar while dragged;
  minimizing an app's last window leaves the app active with no key window; the shell's
  activation (the Dock) brings back the latest minimized window; a click outside an
  open menu closes it and is consumed. Headless tests in `tests/window.rs`; the guest
  desktop test saves `m15-terminal-full-screen`, `m15-terminal-windowed` and
  `m15-terminal-minimized`. Not done: title bars (the toolkit draws none yet, so our
  own apps cannot be dragged; macOS precedent is app-drawn title bars with the traffic
  lights), `xdg-decoration`, layer-surface popups beyond placement, snapping (phase 8).
- [x] **M16 Firefox on Wayland** (`desktop/milestone-16`, CI green in run 40784918925)
  The guest image carries `firefox-esr` (no X server, no XWayland); `desktop::firefox_runs_on_wayland_and_opens_a_context_menu`
  starts it on the compositor's socket with `MOZ_ENABLE_WAYLAND=1`, waits for a page colour
  on the CRTC, opens the page's context menu (an xdg popup) and takes the window full
  screen, saving `m16-firefox-page`, `m16-firefox-context-menu` and
  `m16-firefox-full-screen`. No new protocol was needed for this; `cursor-shape-v1` waits
  for themed cursors (until then clients draw their own cursor surfaces) and
  `xdg-decoration` for server-side title bars. The guest opens a local page (Firefox does
  not render a `data:` URL from its command line) with first-run pages turned off.
- [x] **M17 Lock screen and idle** (`desktop/milestone-17`, CI green in run 40786134854)
  `ext-session-lock-v1` (only shell clients see it, as only loginwindow locks a Mac):
  locked after a blank frame, only the lock surface drawn and given input, focus
  restored on unlock, blank and still locked if the locker dies, a new locker may take
  over a dead one's lock, Super shortcuts off while locked. `unbound-lock`: date, time,
  initial, name and a secure field, PAM (`/etc/pam.d/unbound-lock`, libpam loaded at run
  time) off the UI thread, a rejected password clears and shakes the field (macOS).
  `Ctrl+Super+Q` (macOS Control-Command-Q) and logind's `Lock` signal start it; the
  compositor sets logind's `LockedHint`, which `unbound-power` waits for before sleep.
  `ext-idle-notify-v1` v2 and `idle-inhibit-v1` (a visible inhibitor holds off ordinary
  idle, not input idle). Toolkit: `SecureField`, `Role::Lock`. Guest screendumps
  `m17-lock-screen`, `m17-lock-rejected`, `m17-unlocked`. `unbound-power` watches
  `ext-idle-notify` (one notification per policy threshold) to dim, lock and sleep, and
  the compositor exports `WAYLAND_DISPLAY`, `XDG_CURRENT_DESKTOP` and `XDG_SESSION_TYPE`
  to systemd user units and D-Bus activation. Not done: dimming has no backlight to act
  on yet; the lock screen shows no wallpaper and no battery or network state.
- [ ] **M18 Full-screen screenshots** (`desktop/milestone-18`; ci/vm not green yet)
  `ext-image-capture-source-v1` (output sources) and `ext-image-copy-capture-v1` into
  `wl_shm` buffers, offered to shell clients only (on a Mac capture needs the Screen
  Recording permission; here the shell is the only trusted capturer and apps will go
  through the portal). The cursor is left out unless `paint_cursors` is asked for, as in
  macOS screenshots; a locked screen captures only the lock surface; pointer cursor
  sessions report `stopped`. `unbound_shell_v1` v2 sends `capture_screen` on
  Super+Shift+3 (Command-Shift-3), matched on the unshifted key so it holds on any layout.
  `unbound-screenshot`, a shell client the compositor keeps running, writes
  `Screenshot YYYY-MM-DD at HH.MM.SS.png` (macOS's name) into the XDG desktop folder
  through a hidden partial file, numbering a taken name ` (2)`. Toolkit:
  `Window::capture_screen`. Guest screendumps `m18-screenshot`,
  `m18-screen-before-capture`. Not done: the floating thumbnail, Super+Shift+4 region and
  window selection, Super+Shift+5, the shutter sound, dma-buf capture targets, toplevel
  sources, and the portal's `Screenshot` backed by this capture.
- [ ] **M19 Area screenshots** (`desktop/milestone-19`; ci/vm not green yet)
  Super+Shift+4 (Command-Shift-4) sends `capture_area`; `unbound-screenshot` shows a
  full-screen overlay with a crosshair, the dragged-out selection and its size, then on
  release hides it, captures the screen and saves the area like a full-screen shot.
  Escape cancels, as on a Mac; a click without a drag takes no picture. Guest screendumps
  `m19-area-selection`, `m19-area-screenshot`, `m19-after-area-capture`. Not done: Space
  for window selection (needs toplevel capture sources), the floating thumbnail, a
  crosshair cursor shape in place of the drawn one.
- [ ] **Explicit sync: `linux-drm-syncobj-v1` (P0)**
  Acquire points through `DrmSyncPointBlocker`, release points from frame completion.
  *Done when* guest tests show a commit is not presented before its acquire point
  signals and the release point signals after the buffer leaves use.
  Also wait for a dma-buf's implicit fence (`DmabufBlocker`) before latching a commit
  from a client without explicit sync.
  Deferred behind the screenshot and app milestones: neither vkms nor virtio-gpu in the
  guest's 6.12 kernel offers DRM syncobjs (virtio-gpu gains them in a later series), so the guest
  cannot test the protocol; the implicit-fence half can be tested now with `sw_sync`
  fences imported into a udmabuf (`DMA_BUF_IOCTL_IMPORT_SYNC_FILE`).
- [ ] **Input methods** (`desktop/input-method`) phase 4: the compositor offers
  `zwp_text_input_v3`, `zwp_input_method_v2` and `zwp_virtual_keyboard_v1` (what Fcitx5 and
  IBus use) through Smithay, which moves text-input focus with keyboard focus. Candidate
  windows are popups of the focused surface, placed below the text being composed and
  flipped above it when the screen ends below, kept on screen horizontally (as macOS places
  its candidate window). The toolkit binds text-input v3 on its seat: preedit and commit
  strings become `Event::Preedit` and `Event::Commit`, applied on `done`; a text field
  shows the preedit underlined through Parley's compose support and reports its
  composition area, which the run loop sends as the cursor rectangle (`App::text_input_area`)
  and which enables and disables text input as focus moves; secure fields report none, so
  passwords never reach an input method. Tests: committed text reaches the focused client,
  keys go to the input method while it grabs the keyboard, both candidate placements, and a
  toolkit text field receiving composed Japanese. Not done: starting Fcitx5 with the
  session; surrounding text and content type; while an input
  method grabs the keyboard, an xdg popup's keyboard grab is refused (Smithay's popup grab
  check), so menus opened from a focused text field close at once.
- [ ] **Toolkit on Vello and Parley** (track; moved into `desktop/integration`)
  `vello_cpu` first frame into `wl_shm`, `vello_gpu` afterwards, Parley text; goldens
  re-derived from scene descriptions; panel, launcher and terminal move over; a toolkit
  sample app whose start time and RSS `ci/perf` measures.
  Track `desktop/track-toolkit` (merged with M12 for the compositor's damage handling).
  Done: Vello CPU and GPU renderers for fills, strokes, gradients, rounded rectangles, clips
  and shadows; Parley text in Inter with fallback; goldens from exact coverage, both
  renderers within 2/255; `Window` with the first frame on the CPU in `wl_shm` and later
  frames on a Vulkan surface; `unbound-sample` measured by `ci/perf`; widgets (stack, label,
  button, switch, slider, text field with the section 1.5 bindings) with AccessKit nodes and
  actions. Paused at 798fa67: lint, tests and clippy pass locally (`--locked`); CI for
  that commit was not awaited (earlier commits: lint/test/deb green at bd3c763).
  Next, in order: `wl_seat` pointer and keyboard in `Window` (xkbcommon keymap, map to
  `ui::Event`; test with `Headless::move_pointer_to`/`press_button`/`key`); drive a `Ui`
  from `Window` and move `unbound-sample` onto widgets; publish `Ui::accessibility` on
  AT-SPI with `accesskit_unix` (test on a private bus via `unbound-test-bus`); scroll view
  on `unbound-spring` and tab view; damage-limited redraw with an exact-metric bench;
  `wp_fractional_scale_v1`.
- [ ] **Session lock and idle** `ext-session-lock-v1` with blank fallback,
  `ext-idle-notify-v1`, `idle-inhibit`, driven by the `idle` config.
- [ ] **Watchdog** `wl_display` ping heartbeat, unresponsive marking after 3 s.

## Desktop services track (`desktop/track-services`)

Built beside M9–M14 without the screen work; each item is headless and tested on
private buses.

Paused at `adfee74`. Run 40687701440 (automount, d3a6bfe) was still queued, so
`unbound-tray`, `unbound-automount` and `ci/portal` (its `download-artifact@v7`) are
green locally but not yet on CI; check that run first. `ci/perf` failures are the
main track's. Next: the unchecked items below, then theming propagation (GTK
`settings.ini`, cursor theme) and Do Not Disturb persistence.

- [x] Notification presentation model: banners and Notification Center, stacking by
  app, timeouts, urgency, `transient`, actions, Do Not Disturb (`unbound-notify`).
- [x] Writable `org.unbound.Config` that edits `config.kdl` in place, the
  `unbound-config` session daemon, and the invalid-file notification.
- [x] `unbound-power`: lock before sleep through a logind delay inhibitor, UPower
  battery alerts, `org.freedesktop.ScreenSaver` turned into logind idle inhibitors;
  the idle state machine as logic.
- [x] `unbound-polkit` agent protocol, tested against Debian 13's polkitd (`ci/polkit`).
- [x] Packaging of `unbound-config` and `unbound-power` as systemd user units with
  D-Bus activation, checked in `ci/deb`.
- [x] `xdg-desktop-portal-unbound`: live `Settings`; `FileChooser`, `Screenshot` and
  `ScreenCast` protocols over UI and capture traits; `ci/portal` against Debian 13's
  frontend.
- [x] `unbound-tray` (`org.kde.StatusNotifierWatcher`) and `unbound-automount`
  (UDisks2 mounting, "not ejected properly" warning).
- [x] `unbound-network` (NetworkManager: Wi-Fi power, network list, joining with a
  password, wired status) and `unbound-bluetooth` (BlueZ: power, paired and nearby
  devices, connecting, pairing through a `KeyboardDisplay` agent), tested against
  D-Bus fakes that follow the published APIs.
- [x] `unbound-audio` (PipeWire: outputs, inputs, default device through WirePlumber's
  metadata, volume and mute on the device route with `save`, the volume keys in
  sixteenths and Option-Shift quarter steps), guest-tested against pipewire and
  wireplumber with null devices.
- [ ] Volume keys from the compositor (`XF86AudioRaiseVolume`, `LowerVolume`, `Mute`)
  to the shell, the on-screen level indicator, and Control Center–style quick toggles
  for Wi-Fi, Bluetooth and sound over the three clients; the Wi-Fi password and
  Bluetooth passkey prompts. Menu bar and Settings integration after M23 and M25.
- [ ] Prove per-device volume memory across a WirePlumber restart on a device with
  routes (null devices have none; try `snd-dummy` or `snd-aloop` in the guest kernel).
- [ ] Register `FileChooser`, `Screenshot` and `ScreenCast` once the toolkit dialogs
  and the compositor's capture (PipeWire) implement their traits.
- [ ] Resolve `appearance "automatic"` to dark or light on a schedule; the portal
  reports no preference until then.
- [ ] Drive the idle machine from `ext-idle-notify-v1` once the compositor offers it
  (M17), dim through the backlight, and count logind `idle` block inhibitors of
  other programs (`BlockInhibited`) as inhibition.
- [x] `idle suspend-after`, by default when the display turns off as on a desktop
  Mac, carried out by `unbound-power` through logind.
- [ ] Toolkit surfaces: notification banners and center, the polkit dialog (then
  package `unbound-polkit`), each drawing the existing model.
- [ ] Notification polish that waits for the UI: keep a banner while the pointer is
  over it, as macOS does; persist Do Not Disturb across daemon restarts; bound the
  center per app.

## Needs hardware

Kernel steps owned by the user: K1 boot the M2 mini to userspace, K2 `simpledrm` on the
boot framebuffer, K3 USB input. Later, the Apple DCP and agx drivers unlock:

- Overlay planes and direct scanout on Apple's display controller.
- `VK_KHR_external_semaphore_fd` release path and frame timings on Honeykrisp.
- Zero-copy sampling of dma-bufs through `VK_EXT_image_drm_format_modifier` (and
  linux-dmabuf feedback with tiled modifiers) on Honeykrisp, instead of the copy.
- Input-to-photon measured with a camera or photodiode on a real Mac.

## Noticed in passing

- The frame clock predicts with the maximum of the last 16 deadline-to-done times, so
  one late wake-up moves every deadline earlier for 16 frames (p50 latency rose from
  6.7 to 11.4 ms on the runners); a high percentile would track it with less latency.

- `unbound-render` uploads through a staging buffer; `VK_EXT_host_image_copy` (on
  lavapipe) would copy straight from the shm pool into the texture.
- `unbound-bench` reuses `unbound-test-wayland`, whose helpers panic on failure; fine for
  a measuring tool, but its errors read as test assertions.
- Overlay-plane assignment is enabled (vkms has eight overlays) but no test puts a
  non-fullscreen window on one yet; a guest test with a dma-buf window above a composed one
  would cover it.
- Smithay logs and skips a surface whose buffer fails to import; the compositor has no
  tracing subscriber, so such failures are silent. Install one (stderr, `RUST_LOG`) in
  `main`.
- DrmCompositor's plane tests exclude modesets, so the first frame after taking the
  output is always composed.
- `image-compare` 0.5.0's SSIM sums the window's (co)variances instead of averaging them,
  so its stabilising constant is 64 times too small and near-flat windows score low: two
  gradients differing by 1 per channel scored 0.968. The toolkit's tests compute SSIM
  themselves; `unbound-compositor`'s golden test still uses the crate.
- `unbound-term`'s goldens use DejaVu Sans Mono from `fonts-dejavu-core`, which
  `ci/lavapipe-test` installs.
- The first-window focus stand-in (M13) is still in place; apps started from the terminal
  carry no activation token yet.
- `ci/perf` fails on `frame_typical_p99` (16–23 ms against 8.33 ms) and once
  `frame_full_p99` while other jobs share the runners' Mac mini; the services branch
  does not touch the compositor, and milestone-8's tip passed with the runners idle.
- `unbound-lock` keeps the password in a `String` until PAM's copy is zeroed; zeroize the
  field's buffer too.
- The lock screen ignores logind's `Unlock` signal (`loginctl unlock-session`); decide
  whether an administrator's unlock should be honoured.
- `ci/lavapipe-test` and `ci/vm` sometimes lose the linker to the OOM killer (`ld terminated
  with signal 9`) when two runs share the runners' Mac mini; a rerun passes. Limiting link
  jobs or giving the runners more memory would end it.
- Since `unbound-screenshot` joined the session (M18), ci/vm fails intermittently in the
  typing tests: run 40791212426 the lock test (desktop not back), 40793405279 the launcher
  test (typed command not run), 40793466947 the lock test (`unbound-lock` still running
  after a good password) and the M19 area test (no overlay in `m19-area-selection`,
  no file seen although the tool logged a save). The headless tests and the M18
  full-screen guest test pass. Suspects: a third hidden exclusive overlay layer and the
  compositor's single `exclusive_layer` slot, or guest CPU load from a third toolkit
  process; the next step is a headless test with two hidden exclusive shell layers.
- PipeWire 1.4.2 ignores a Props change on a suspended adapter node that has no device
  (a null sink nobody plays to): `pw-cli s <sink> Props '{ channelVolumes: [ 0.5, 0.5 ] }'`
  leaves the volumes at 1.0, while the same change on the running sink applies.
  `unbound-audio` sets such sinks' volume through the node, so its change is lost the
  same way; raise it upstream.
- A null-audio-sink given `media.class = "Audio/Source"` stalls WirePlumber 0.5.8's event
  queue when it activates the node, after which no default node is chosen; virtual
  sources need `Audio/Source/Virtual`.

## Resume here (desktop agent 1)

- Branch `desktop/ci-debian` (procps for `kill` in ci/lavapipe-test) and `ci/shared-vm-cache`
  were pushed on top of main bb05be5. Their CI runs were in flight when work paused: check
  them and merge each into main when it is fully green, then delete `ci/shared-vm-cache` and
  its worktree `desktop-wt/vm-cache`.
- Next in the chain: `desktop/milestone-24`. Its local merge of main (f814e1e, before bb05be5)
  passed all 16 desktop guest tests on the laptop; the 2 audio failures there are fixed
  by 3fc122a. Merge main into it again, push, merge on green, then M25 and M26.
- Then phase 7: dconf writer, animated and HiDPI cursors.
