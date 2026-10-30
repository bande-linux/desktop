# Rendering Architecture: Decision Record

Status: **decided** (October 2026). Scope: how the compositor composes frames, how the
toolkit and apps draw, and which performance numbers every change is held to.

## Context

- Unbound targets Apple Silicon. The Asahi GPU stack ships Honeykrisp, a Vulkan 1.4
  conformant Mesa driver [5]. Until the agx kernel driver is usable in our images,
  everything renders on lavapipe (Mesa's CPU Vulkan) on the CI runners and in VMs.
- The runners (Ubuntu 24.04, Mesa 25.2.8, probed on 2026-10-28) expose lavapipe with
  Vulkan 1.4, `VK_EXT_external_memory_dma_buf`, `VK_KHR_external_memory_fd`, timeline
  semaphores, `synchronization2`, dynamic rendering and `VK_EXT_host_image_copy`. They
  have no `VK_EXT_image_drm_format_modifier`, no `VK_KHR_external_semaphore_fd`, no
  `/dev/dri` and no `/dev/udmabuf`, but they do have `/dev/kvm`.
- `linux-drm-syncobj-v1` (explicit sync) is P0. Its acquire and release points are DRM
  timeline syncobjs, the kernel object that Vulkan timeline semaphores are modelled on
  and that converts to and from `sync_file` fds [3][4].

## Decision 1: the compositor renders with its own Vulkan renderer on `ash`

A new crate, `unbound-render`, owns a Vulkan 1.3 device through `ash` and is the only
renderer in the compositor. Smithay stays the kernel and protocol layer.

| Need | `wgpu` | Smithay GLES renderer | Own Vulkan (`ash`) |
| :--- | :--- | :--- | :--- |
| dmabuf import | wgpu-hal only, unsafe, single plane, needs modifiers [1] | yes, through EGL | yes, any plane count, LINEAR without modifier extension |
| Explicit sync | no external semaphore or `sync_file` import/export in the API [1] | EGL native fences | timeline semaphores and `sync_file` directly |
| Direct scanout, overlay planes | not applicable | `DrmCompositor` assigns planes [2] | `DrmCompositor` assigns planes once the renderer implements Smithay's renderer traits [2] |
| Control of submission, memory, queues | hidden behind portability layer | GLES 2 model | full: persistent textures, partial uploads, async compute for blur |
| Availability on Apple Silicon | via Vulkan | Asahi GL 4.6 / GLES 3.2 | Honeykrisp Vulkan 1.4 [5] |

Smithay ships GLES, pixman and multi-GPU renderers but no Vulkan renderer [6], and
`wgpu` would hide exactly the parts that decide latency: when a buffer is read, which
fence is waited on, and whether a frame is repainted at all.

Design rules for `unbound-render`:
1. **Persistent textures.** A surface's texture lives as long as the surface; a commit
   uploads only the buffer's damaged rows (`wl_shm`) or rebinds an imported image (dmabuf).
2. **Damage-limited repaint.** Each frame repaints only the union of damaged output
   regions, scissored; nothing outside it is shaded. Readback exists only for tests and
   screenshots.
3. **Explicit sync without stalls.** Acquire points are waited for on the CPU with
   Smithay's `DrmSyncPointBlocker` before a commit is latched [3], so a late client never
   blocks the compositor's queue. Release points are signalled from the frame's
   completion: by `sync_file` export from a Vulkan semaphore where the driver has
   `VK_KHR_external_semaphore_fd` (Honeykrisp), otherwise from the CPU when the frame's
   fence signals (lavapipe).
4. **Smithay renderer traits.** For the DRM backend the renderer implements Smithay's
   `Renderer`, `ImportDma` and `ImportMem` traits so that `DrmCompositor` keeps doing
   plane assignment, direct scanout and overlay planes; elements that fail the atomic
   test are composed by us on the primary plane [2].

Validation that needs hardware is designed in, not assumed: plane assignment and
direct scanout are tested on VKMS (which has overlay planes) in a KVM guest; whether
Apple's display controller exposes usable overlay planes, and the semaphore `sync_file`
path, are validated on Honeykrisp when the rig exists. The CPU release path covers the
case where they are not.

## Decision 2: the toolkit draws with Vello and lays out text with Parley

`unbound-toolkit` replaces its rectangle-and-atlas renderer with Linebender's Vello
sparse-strips renderers and Parley.

- **Vello** [7]: `vello_cpu` (multithreaded, SIMD, currently the most mature) and
  `vello_gpu` (CPU path preprocessing, GPU rasterisation, no compute shaders required)
  share one scene model. The compute renderer is experimental and is not used.
- **Parley** [8]: rich text layout with shaping by HarfRust, bidi and font fallback,
  which the CJK and accessibility requirements need and which `ab_glyph` cannot do.
- Extending our own renderer would mean rebuilding path filling, anti-aliasing,
  gradients, clips, rounded rectangles and shaping ourselves, with no speed advantage.

How it is used:
1. An app's **first frame is drawn with `vello_cpu` into a `wl_shm` buffer**, so it does
   not wait for Vulkan device creation. Later frames move to `vello_gpu` once its device
   is ready; small damage may stay on the CPU path.
2. Golden images in CI are rendered with `vello_cpu`, which is deterministic on lavapipe
   runners; `vello_gpu` is checked against the same goldens within the SSIM bound.
3. The toolkit's public scene API (`Scene`, `Item`) stays ours, so the backend can change
   without touching apps.

## Decision 3: performance targets, and how CI holds every change to them

Targets on hardware (Apple Silicon, 120 Hz ProMotion panel, refresh interval 8.33 ms):

| Metric | Target |
| :--- | :--- |
| Compositor latency: client commit to scanout | a commit latched before the frame deadline is on screen at the next vblank (≤ 1 refresh interval) |
| Input to photon, responsive app | ≤ 2 refresh intervals (16.7 ms) |
| Compositor frame, typical desktop (8 windows, one updating) | CPU ≤ 1 ms p99, GPU ≤ 2 ms p99 |
| Rendering deadline | start composing at vblank − (predicted render time + 1 ms), not at vblank |
| App start, toolkit app, warm caches | exec to first committed frame ≤ 100 ms |
| Memory | compositor ≤ 64 MB RSS with 8 windows; toolkit app ≤ 30 MB RSS |

CI cannot measure photons or a real GPU, so `ci/perf` measures on lavapipe with
`unbound-bench` on every push to `main` (and on a branch commit whose message contains
`[perf]`) and splits metrics into two kinds:

- **Exact metrics**, independent of machine load, compared exactly: pixels repainted per
  frame equal the damaged area; bytes uploaded per frame equal the damaged rows; frames
  between a commit before the deadline and its presentation equal 1 on a virtual 120 Hz
  clock. The frame scheduler reads time through an injected clock, so the scheduling
  tests run on virtual time that does not move while a frame renders and are
  independent of the runners' load.
- **Real-clock presentation metrics**, reported against the targets but not gated:
  commit-to-present latency p50/p99 (target ≤ 2 refresh intervals) and the number of
  frames presented after the vblank their deadline promised, for commits at 100 phases
  of a real 120 Hz clock. On a host shared by five runners, a sleeping thread's wake-up
  is late by more than the 1 ms margin often enough that a gate would only measure the
  neighbours.
- **Timed metrics**, reported against budgets but not gated: typical frame at 1920×1080
  p99 ≤ 8.33 ms (lavapipe must keep 120 Hz when damage is small), full repaint of 8 windows
  p99 ≤ 16.7 ms, median of five compositor starts to listening socket ≤ 500 ms. The runners
  share one 8-core host with other repositories' kernel builds and hardware jobs, and under
  that load the unchanged renderer's typical-frame p50 moved from 0.28 ms to 3 ms; a wall-clock gate
  would measure the neighbours, not the change. The summary prints the host's load average
  next to them. They become gates again when they run on a runner reserved for measurement.
- **Gated metrics**: every exact metric above, and compositor idle RSS ≤ 128 MB (lavapipe's
  LLVM JIT included). Each run writes `perf.json` as an artifact and a table to the job
  summary so trends are visible per commit. The exact metrics also count pixels shaded,
  which an opaque window above a damaged area keeps to that area.
- **Toolkit sample** (`crates/testing/sample`, measured by `unbound-toolkit-bench` in the
  same job): exec to first committed frame, median of five starts against the 100 ms target
  (reported); resident memory when the first frame is mapped and after the GPU path is up,
  against the 30 MB target (reported, since lavapipe's LLVM alone adds about 100 MiB); the
  same app kept on the CPU path with `UNBOUND_RENDERER=cpu`, which is the toolkit's own
  memory (gated at 30 MiB); and the sample scene's frame time on `vello_cpu` and on
  `vello_gpu` including readback (reported).
- **Idle wakeups** (`unbound-idle`, same job, gated): a whole session runs headless on a
  private session bus (the compositor in-process, the shell, the clipboard and screenshot
  daemons, every session service, and Files, Settings, Terminal and Activity Monitor) and
  sits untouched for 60 s three times: windows open, windows minimised, screen locked. Each
  process's context switches, summed over its threads, count as its wakeups (lavapipe's
  `llvmpipe-*` rasteriser threads, which a GPU does not have, are listed but not counted);
  every process must stay at or under 1 per second, so a timer, a poll or a run loop that
  spins while nothing happens fails the gate. The one exception is live data on screen:
  with the windows open, Activity Monitor redraws its sample every 2 s and the compositor
  shows those frames, so they may wake 5 and 10 times a second. The clocks' minute ticks
  and Activity Monitor's sampling while hidden fit inside the budget.

Measuring true input-to-photon needs a camera or photodiode on a real Mac; that is a
hardware-rig task. Until then the exact frame-latency metric is the gate.

## Consequences

- The compositor stops depending on `wgpu`; the toolkit depends on Vello (which uses
  `wgpu` for `vello_gpu`). The two processes no longer share a GPU abstraction, which they
  never needed: they share dmabufs and syncobjs through the protocol.
- A KVM guest with VKMS becomes the test bed for the DRM backend, plane assignment and
  explicit sync, since the runners have `/dev/kvm` but no DRM device.

## Sources

1. wgpu-hal 30, Vulkan `Device::texture_from_dmabuf_fd`: <https://docs.rs/wgpu-hal/30.0.0/wgpu_hal/vulkan/struct.Device.html>
2. Smithay `backend::drm::compositor` (plane assignment, direct scanout): <https://docs.rs/smithay/latest/smithay/backend/drm/compositor/index.html>
3. Smithay `wayland::drm_syncobj` (blocker, release on buffer drop): <https://docs.rs/smithay/latest/smithay/wayland/drm_syncobj/index.html>
4. wlroots `drm_syncobj.h` (timelines modelled on Vulkan timeline semaphores, `sync_file` conversion): <https://wlroots.pages.freedesktop.org/wlroots/wlr/render/drm_syncobj.h.html>; protocol: <https://isaacfreund.com/docs/wayland/linux-drm-syncobj-v1>
5. Alyssa Rosenzweig, "Vulkan 1.4 sur Asahi Linux": <https://rosenzweig.io/blog/vulkan-14-sur-asahi-linux.html>
6. Smithay `backend::renderer` (GLES, pixman, multi-GPU; no Vulkan): <https://docs.rs/smithay/latest/smithay/backend/renderer/index.html>
7. Vello repository (sparse strips, `vello_cpu`, `vello_gpu`, status): <https://github.com/linebender/vello>
8. Parley: <https://docs.rs/parley/>
9. Linux kernel, multiplane overlay constraints (example of per-driver plane limits): <https://docs.kernel.org/next/gpu/amdgpu/display/mpo-overview.html>
