# Backlog

Ordered milestones derived from `docs/ARCHITECTURE.md` (phases and protocol matrix) and
`docs/RENDERING.md`. Each milestone lives on `desktop/milestone-N`, stacked on the one
before, and is done when its definition of done holds on the self-hosted runners
(`ci/lint`, `ci/lavapipe-test`, `ci/deb`, and the jobs it adds) without real GPU or DRM
hardware. Things noticed in passing go to the end of this file, not into a diff.

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
- [ ] **M11 First boot on the M2 Mac mini (hardware smoke test)**
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
  *Status (2026-10-28)*: blocked on K1. The latest `hil` run (`kis-2`, 10:13) ends at
  m1n1's "Vectoring to next stage" with no kernel output. Once K1 and K2 land, under the
  shared lock (`mkdir hil-host/mini.lock`, short sessions): boot the `ci/vm` root image
  contents plus the `ci/deb` packages, start seatd and run the `guest` test binary's
  `scanout::a_composed_window_is_scanned_out_on_vkms` with the card chosen by connected
  connector instead of by the vkms driver name, then release the lease and the lock.
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
- [ ] **M13 Shell protocols** `wlr-layer-shell`, `xdg_activation_v1` (replaces the
  first-window focus stand-in), `wp_viewporter`, `wp_fractional_scale_v1`. *Done when*
  a layer surface reserves its exclusive zone and toplevels are placed outside it.
- [ ] **M14 Minimal shell: first usable desktop**
  `unbound-panel` (left edge by default, from `config.kdl`) with app buttons and a
  clock, `unbound-launcher` on `Super+Space` listing `.desktop` entries, and a minimal
  `unbound-term` (PTY, scrollback, `Super+C`/`Super+V`), all on the current toolkit.
  *Done when* golden tests cover each surface, a guest session test launches the
  terminal from the launcher and types into it, and the same works on the mini.
- [ ] **M15 `linux-drm-syncobj-v1` explicit sync (P0)**
  Acquire points through `DrmSyncPointBlocker`, release points from frame completion.
  *Done when* guest tests show a commit is not presented before its acquire point
  signals and the release point signals after the buffer leaves use.
  Also wait for a dma-buf's implicit fence (`DmabufBlocker`) before latching a commit
  from a client without explicit sync.
- [ ] **M16 Toolkit on Vello and Parley**
  `vello_cpu` first frame into `wl_shm`, `vello_gpu` afterwards, Parley text; goldens
  re-derived from scene descriptions; panel, launcher and terminal move over; a toolkit
  sample app whose start time and RSS `ci/perf` measures.
  Track `desktop/track-toolkit`. Done: Vello CPU and GPU renderers for fills, strokes,
  gradients, rounded rectangles, clips and shadows; Parley text in Inter with fallback;
  goldens from exact coverage, both renderers within 2/255.
- [ ] **M17 Session lock and idle** `ext-session-lock-v1` with blank fallback,
  `ext-idle-notify-v1`, `idle-inhibit`, driven by the `idle` config.
- [ ] **M18 Watchdog** `wl_display` ping heartbeat, unresponsive marking after 3 s.

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
- `ci/lavapipe-test` installs `fonts-dejavu-core`, which no test uses any more.
