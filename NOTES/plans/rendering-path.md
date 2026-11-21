# Rendering path

Audit gap 18. Plan only for what is left; delete stages as they land.

## What the user gets

- Colours that are right on a wide-gamut monitor, without a setting.
- Dragging, animation and Glass with room to spare at 120 Hz.
- Video and browser frames shown without a copy, and screen sharing without a CPU copy.

## What happens to a pixel today

From the code:

| Step | Who | Applies to |
|---|---|---|
| Toolkit kept texture to swapchain image | client GPU | toolkit GPU windows |
| shm to staging | compositor CPU | shm (first frames, CPU renderer) |
| dmabuf or staging to surface texture | GPU | shm, LINEAR |
| Draw the quad | GPU | all |
| Compositor image to the display buffer | GPU, one copy in the frame's command buffer | all composited frames |

- Tiled client buffers are not accepted: `linear_formats` offers only LINEAR.
- Rounded windows are always composited: `PieceElement` has no `underlying_storage`. Layer surfaces and full-screen windows can reach a plane.
- The compositor thread waits for the GPU on every frame (`submit_and_wait`).
- Each swapchain buffer has its own compositor image, and each surface its own texture: 14.7 MB per 2560x1440 window on top of the client's buffers.

Already built: the output copy on the GPU (the mapping and memcpy remain for CPU renderers, virtual GPUs and a refused import), NV12 video sampled in place, and screen sharing through shared dmabuf or memfd memory with no CPU copy.

**Rounded corners and planes.** A rounded window on a plane needs the composed frame above it with a hole. Smithay 0.7 can do this with underlays, below the primary, which needs a driver whose overlays can sit below it.

**Versions.** Debian 14 (forky), Smithay 0.7.0: no colour management, but underlays and a VRR API. No Mesa or kernel blocker exists for stages 5 and 6.

**Others.** KWin 6.5 has overlays, off by default because of driver freezes; its author measured 7.3 W against 8.4 W for video on a laptop, and overlays are off with a colour profile or night light until the KMS colour pipeline is used. GNOME 48 has `wp_color_management_v1`. wlroots has it too (from memory).

## macOS

- IOSurface shares one buffer across processes and GPU without a copy (from memory).
- Metal apps can bypass the compositor when eligible, "direct to display" (WWDC 2017 via secondary sources).
- `CAMetalLayer` exposes `colorspace`, `wantsExtendedDynamicRangeContent` and `edrMetadata`; tone mapping costs GPU and memory (Apple docs).
- `CADisplayLink`/`CAMetalDisplayLink` with `preferredFrameRateRange` drive ProMotion (Apple docs).
- A low-power video path through hardware surfaces is from memory.

## Colour

The correct order is sRGB correct first, then tagged content, then HDR.

- **Built:** frames go through a shader that converts sRGB to the display's primaries from EDID, as in "Colour" in `compositor.md`. It works on any GPU and kernel, and is on by default with a per-display switch. While it is on, nothing is scanned out directly on that display. White and greys stay exact.
- **Optional, where the hardware offers it:** standard KMS `DEGAMMA_LUT`, `CTM` and `GAMMA_LUT` on the CRTC (amdgpu, i915) can do the same conversion with planes kept. Not built: it needs verifying against black screens on such hardware. It has to agree with Night Light, which uses the gamma ramp today.
- **HDR:** needs the connector's `Colorspace` and `HDR_OUTPUT_METADATA`. Changing colour mode is a modeset, so HDR means a per-display Settings toggle, as on a Mac. Last.

## Variable refresh

Not built: the compositor reads no `vrr_capable` and Settings has no switch. Bring the switch back when the compositor uses the connector's `vrr_capable` and Smithay's VRR API (`rendering-hard.md` V).

## Stages, by benefit per risk

| # | Stage | Gain | Risk | Blocker | Size |
|---|---|---|---|---|---|
| 5 | Sample LINEAR dmabufs, then tiled and compressed | one copy per window, 4 bytes/px per window | medium, **tearing** | stage 6 for Firefox-grade sync | M-L |
| 6 | Stop blocking on the GPU | compositor thread free during GPU work | medium, **freeze** if a fence never signals | none | M |
| 7 | `wp_color_management_v1`, P3, 10-bit | Firefox and players show P3 correctly | low to medium | Smithay has no support, the XML is in `wayland-protocols` | M-L |
| 8 | Windowed video on a plane | KWin's ~1 W on a laptop | **high: driver faults, black screen** | an underlay-capable plane | M |
| 9 | HDR | PQ on HDR monitors | high | stage 7; `Colorspace` and `HDR_OUTPUT_METADATA` from the driver | L |

5. **Direct window textures.** The trap: sampling a client's buffer means the client must not write until release, and the compositor must wait for its fence. Without that, windows tear. Closing animations and stretch-on-resize must hold the buffer. Verify `linux-drm-syncobj-v1` first. Tiled buffers may not suit a plane, so feedback must stay per surface: scanout modifiers for what is on a plane, compressed for the rest. Firefox then gets the same import path.
6. **Asynchronous frames.** Export a `sync_file`, wait in the event loop, record render time at fence signal. A fence that never signals must go through the existing GPU-loss recovery.
7. **Tagged content.** Per-surface description, conversion in shaders or a per-plane colour pipeline where the driver has one; 10-bit composition removes banding under Glass, where the link carries 10 bits.

### Left for the owner's optimisation pass

- Frame clock: the max of 16 stays until there is a camera number. One 8 ms frame moves the latch about 5 ms earlier for 133 ms at 120 Hz; a decaying window may be better, but is a guess until measured.
- Glass: reuse a blur until its backdrop changes, smaller blur passes, the neighbour's redo.
- Ten wake-ups per client update.
- One compositor image shared across the swapchain (saves up to about 45 MB at 1440p).
- Rendering straight into the scanout buffer when no blur is on screen.
- A cursor on a plane.
- shm without a copy (`udmabuf`, as KWin does): the toolkit uses shm only for first frames.

### Not a problem

Text at 1x: the audit found it even and a little soft, not defective, and macOS also renders unhinted and greyscale (from memory). No change unless a side-by-side shows otherwise.

## Portability

Everything built here runs on Debian's own kernels and any GPU through atomic KMS, and every runtime check falls back to the earlier path.

- **Output copy:** any real GPU (integrated or discrete) that imports the dumb buffer as a Vulkan buffer. A refused import, a CPU renderer or a virtual GPU keeps the CPU copy. It assumes the GPU's writes reach the display without CPU-side cache maintenance, which holds for imported external memory on Mesa drivers. Not yet verified on amdgpu, i915 or Xe.
- **NV12 sampling:** needs `VK_EXT_image_drm_format_modifier`, `VK_EXT_queue_family_foreign`, and the driver reporting importable LINEAR NV12 (checked at start). A driver needing a larger stride refuses the import, and video copies again.
- **Screen sharing:** standard `ext-image-copy-capture` dmabufs, the render node from `VK_EXT_physical_device_drm`, `/dev/udmabuf` (Debian's kernel has it), and PipeWire. Without a render node or udmabuf it uses `wl_shm` on the same memory.
- **Colour:** EDID and a shader, with no kernel property needed.

The frame scheduler (`rendering-hard.md` T) learns flip times, render times and commit lead per output, per machine, so it suits any driver without per-GPU constants.

## Owner steps

1. **Colour after installing.** Reds and greens look calmer on a wide-gamut monitor. Compare against the monitor's own sRGB mode; Settings ▸ Displays ▸ Colour profile ▸ None turns it off, and sRGB, Display P3 or an added ICC profile replace the EDID's. Set the monitor back to its wide default first, or it is converted twice.
2. **Latency with a phone.** 240 fps slow motion, framing a key and the screen, in Terminal at 120 Hz. Ten presses; count frames from the key bottoming out to the glyph (each 4.2 ms). Add the same with a mouse button and a menu. The mouse and keyboard's own delay is included, so it compares only against the same run on macOS. Ten minutes, before and after stage 6.
3. Drag a large window in Glass on the real display; the frame numbers need the real path.

## Not measured

- Stage costs on the build machine's GPU and on amdgpu, i915 and Xe.
- Client-side copy, per-window memory in practice, DRAM bandwidth, hardware decode, windowed video on a plane, input-to-photon latency.

## Sources

- [KWin overlays, Phoronix](https://www.phoronix.com/news/Plasma-6.5-Overlay-Planes-KWin), [merge request](https://invent.kde.org/plasma/kwin/-/merge_requests/7795), [XDC 2025 slides](https://indico.freedesktop.org/event/10/contributions/424/attachments/275/363/XDC2025.pdf)
- [GNOME 48 colour management](https://www.phoronix.com/news/GNOME-wp_color_management_v1), [the protocol](https://wayland.app/protocols/color-management-v1)
- Apple: [CAMetalLayer](https://developer.apple.com/documentation/quartzcore/cametallayer.md), [edrMetadata](https://developer.apple.com/documentation/quartzcore/cametallayer/edrmetadata.md), [CAMetalDisplayLink](https://developer.apple.com/documentation/quartzcore/cametaldisplaylink.md), [IOSurface](https://developer.apple.com/documentation/iosurface)
