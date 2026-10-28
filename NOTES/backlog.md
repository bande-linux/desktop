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
- [ ] **M8 Frame scheduling and `wp_presentation_time`**
  Virtual 120 Hz vblank clock in the headless backend, composing at the deadline, frame
  callbacks and presentation feedback after presentation. *Done when* tests show a
  commit before the deadline is presented at the next vblank with the feedback's
  refresh interval and sequence, and `ci/perf` reports commit-to-present frames.
- [ ] **M9 KVM test rig (`ci/vm`)**
  Guest kernel built with `CONFIG_DRM_VKMS` and virtio-gpu, Debian 13 arm64 root image,
  QEMU with `-machine virt,its=off`; the job boots the guest and runs test binaries in
  it. *Done when* `ci/vm` boots and a guest test opens `/dev/dri/card0`.
- [ ] **M10 DRM/KMS backend**
  libseat session, atomic modesetting through Smithay's `DrmCompositor`,
  `unbound-render` implementing Smithay's renderer traits, direct scanout and overlay
  planes. *Done when* the compositor drives VKMS in the guest, a fullscreen opaque
  window is scanned out directly, and the writeback/screendump matches the golden.
- [ ] **M11 `linux-drm-syncobj-v1` explicit sync (P0)**
  Acquire points through `DrmSyncPointBlocker`, release points from frame completion.
  *Done when* guest tests show a commit is not presented before its acquire point
  signals and the release point signals after the buffer leaves use.
  Also wait for a dma-buf's implicit fence (`DmabufBlocker`) before latching a commit
  from a client without explicit sync.
- [ ] **M12 Toolkit on Vello and Parley**
  `vello_cpu` first frame into `wl_shm`, `vello_gpu` afterwards, Parley text; goldens
  re-derived from scene descriptions; a toolkit sample app whose start time and RSS
  `ci/perf` measures.
- [ ] **M13 Shell protocols** `xdg_activation_v1` (replaces the first-window focus
  stand-in), `wlr-layer-shell`, `wp_viewporter`, `wp_fractional_scale_v1`.
- [ ] **M14 Session lock and idle** `ext-session-lock-v1` with blank fallback,
  `ext-idle-notify-v1`, `idle-inhibit`, driven by the `idle` config.
- [ ] **M15 Watchdog** `wl_display` ping heartbeat, unresponsive marking after 3 s.

## Needs hardware

- Overlay planes and direct scanout on Apple's display controller.
- `VK_KHR_external_semaphore_fd` release path and frame timings on Honeykrisp.
- Zero-copy sampling of dma-bufs through `VK_EXT_image_drm_format_modifier` (and
  linux-dmabuf feedback with tiled modifiers) on Honeykrisp, instead of the copy.
- Input-to-photon measured with a camera or photodiode on a real Mac.

## Noticed in passing

- `unbound-render` uploads through a staging buffer; `VK_EXT_host_image_copy` (on
  lavapipe) would copy straight from the shm pool into the texture.
- `unbound-bench` reuses `unbound-test-wayland`, whose helpers panic on failure; fine for
  a measuring tool, but its errors read as test assertions.
