# Rendering path

Audit gap 18. Plan only; nothing here is built. Delete stages as they land.

## What the user gets

- Colours that are right on a wide-gamut monitor, without a setting. Today every sRGB pixel is sent unconverted to a monitor that is not sRGB.
- Dragging, animation and Glass with room to spare at 120 Hz, because the CPU no longer copies frames.
- Video and browser frames shown without a copy, and screen sharing without a CPU copy.

## What happens to a pixel today

Measured on the M2 at 2880x1800 (20.7 MB per frame, whole frame damaged), Honeykrisp, Mesa 26.1.6. A throwaway Vulkan program did each step alone, through `build-run --here`, median of 60 (CPU copies 30). Cost scales with damaged area.

| Step | Who | Full frame | Applies to |
|---|---|---|---|
| Toolkit kept texture to swapchain image | client GPU | not measured | toolkit GPU windows |
| shm to staging | compositor CPU | 1.43 ms | shm (first frames, CPU renderer) |
| dmabuf or staging to surface texture | GPU | 0.56 ms | shm, LINEAR |
| NV12 planes to two textures, always the whole frame | GPU | 0.34 ms 1080p, 0.66 ms 4K | NV12 |
| Draw the quad | GPU | 0.44 ms | all |
| Compositor image to staging | GPU | 0.46 ms | all composited frames |
| Staging to dumb buffer | compositor CPU | 1.55 ms | all composited frames |

- Tiled client buffers are not accepted: `linear_formats` offers only LINEAR.
- Rounded windows are always composited: `PieceElement` has no `underlying_storage`. Layer surfaces and full-screen windows can reach a plane.
- The compositor thread waits for the GPU on every frame (`submit_and_wait`).
- Each swapchain buffer has its own compositor image, and each surface its own texture: 14.7 MB per 2560x1440 window on top of the client's buffers (from the code).

Whole path, one full frame, tight loop including barriers and submit: **3.98 ms today against 1.11 ms** with no copies (into a compressed target; LINEAR adds about 0.1 ms), of a budget of 8.33 ms at 120 Hz.

**The headless numbers leave out the last two rows.** The offscreen target has no export, so the known drag medians (2.7-4.2 ms Refined Solid, 4.5 Wallpaper-Lit, 8.1 Glass) would gain up to 2 ms on a real display. Glass can therefore miss 120 Hz at this size when damage is large. This is inferred from the code and needs confirming on the real display.

Power, SMC DC-in, 8 s per mode, 6 rounds, baseline 6.1 W. The sensor updates at 1 Hz and rounds scatter by about 1 W:

| Load | Today | No copies |
|---|---|---|
| Whole frame at 120 Hz (worst case) | +1.45 W | +0.36 W |
| 1080p30 video in a 1080p area | about 0 W | about 0 W |

So the saving is about 1 W in the worst case, and at most a few tenths of a watt on video, too small to separate from noise. **Copies are not a large power problem on the mini at this load.** Software or hardware decode will dominate video power, and that is not measured here. The gain is CPU time, room in the frame budget, and memory.

Layout matters less than the audit inferred. A full textured pass into each target:

| Layout | Render into | Sample from |
|---|---|---|
| LINEAR | 0.53 ms | 0.61 ms |
| GPU_TILED | 0.54 ms | 0.65 ms |
| GPU_TILED_COMPRESSED | 0.44 ms | 0.44 ms |

That is about 20-30 % of a small number. Bytes reaching DRAM were not measured (no counter found), so compression's power effect on real content is still unknown.

## What the hardware can do

Read from the running kernel and driver, not from documents, except where marked.

**Honeykrisp** offers LINEAR, GPU_TILED and GPU_TILED_COMPRESSED for BGRA8, RGBA8, 10-bit and RGBA16F (sampled, attachment, blend, storage), and for NV12 and P010 (sampled, disjoint). It does not offer INTERCHANGE. Proved by running:
- A LINEAR dmabuf imports as a sampled image.
- A one-object NV12 dmabuf imports as a two-plane image with per-plane views, 1080p and 4K: 0.32 and 0.50 ms for the shader against 0.31 and 0.37 ms from the copies, with the 0.34 and 0.66 ms copies gone.
- An appledrm dumb buffer imports into Honeykrisp both as a buffer (GPU copy 0.45 ms, pixels correct when read back) and as a LINEAR render target (0.51 ms against 0.43 + 0.45).

**DCP.** Three planes. All take NV12, P010, 10-bit and FP16, with LINEAR at a 64-byte pitch, and the lower two also INTERCHANGE. Scaling 1/2x to 4x. Blended surfaces under 32x32 or off screen would crash the firmware, and the kernel rejects them. Plane 0 is the primary at the fixed bottom, and the overlays carry no X formats.
- **Overlays already work.** While writing this, the owner's session had a 341x1440 LINEAR client dmabuf on plane 2 beside the composed primary. `open.md`'s "unverified" is out of date for this case.
- GPU_TILED cannot be scanned out. INTERCHANGE is the only compressed scanout layout, and the kernel supports it, but Mesa 26.1.6's Vulkan driver does not offer it.

**Rounded corners and planes.** A rounded window on a plane needs the composed frame above it with a hole. Smithay 0.7 can do this with underlays, but only below the primary, and plane 0 is the lowest. The fix would be a kernel change making plane 1 the DRM primary. The compositor's frame would then have to be AR24, as the X-format trick works only on the bottom plane.

**Versions.** Debian testing: Mesa 26.1.6-1, `linux-image-arm64` 7.2.15-1. We run our 7.3 series. Smithay 0.7.0 has no colour management, but has underlays and a VRR API. No Mesa or kernel blocker exists for stages 1, 3, 4, 5 and 6.

**Others.** KWin 6.5 has overlays, off by default because of driver freezes; its author measured 7.3 W against 8.4 W for video on a laptop, and overlays are off with a colour profile or night light until the KMS colour pipeline is used. GNOME 48 has `wp_color_management_v1`. wlroots has it too (from memory).

## macOS

- IOSurface shares one buffer across processes and GPU without a copy (Apple's page returned no text; from memory).
- Metal apps can bypass the compositor when eligible, "direct to display" (WWDC 2017 via secondary sources).
- The display coprocessor composes several hardware surfaces, each carrying its own colour space and transfer function. `dcp_surface` has those fields, which supports this (from memory plus our driver's headers).
- `CAMetalLayer` exposes `colorspace`, `wantsExtendedDynamicRangeContent` and `edrMetadata`; tone mapping costs GPU and memory (Apple docs).
- `CADisplayLink`/`CAMetalDisplayLink` with `preferredFrameRateRange` drive ProMotion (Apple docs).
- A low-power video path through hardware surfaces is from memory.

## Colour

The owner's MSI G274QPX is wide gamut. EDID and DCP agree: red 0.674/0.317, green 0.280/0.660, blue 0.143/0.045, `DefaultColorSpaceIsSRGB` false. The link runs as colour mode 100: RGB, 8-bit, signalled sRGB. `get_colorspace` in `plane.c` marks every RGB surface NATIVE, so DCP does no conversion. Unless the monitor's own menu has an sRGB mode on, everything is oversaturated.

The correct order is sRGB correct first, then tagged content, then HDR.

- **Best case:** DCP does it per surface, for free, including on overlays. `dcp_surface.colorspace` has `BG_SRGB`. Whether it converts on an external display is unknown, and only a test boot can tell. The kernel comment and code disagree about it.
- **Fallback:** a matrix from EDID primaries in the compositor's shaders. Overlays then need turning off for anything not already correct, as KWin does. The DCP CTM (Night Light uses it) is one global matrix, in an unknown space.
- **HDR:** DCP lists PQ modes on this monitor at 120 Hz (mode 127: 12-bit YCbCr 4:4:4, BT.2020, max 409 nits, frame average 391). appledrm exposes only CTM: no `Colorspace`, no `HDR_OUTPUT_METADATA`, no `COLOR_PIPELINE`. Changing colour mode is a modeset (about 0.5 s of black in `dmesg`). So HDR means a per-display Settings toggle, as on a Mac, and a kernel project. Last.

## Variable refresh: no switch until the kernel reports it

- The compositor reads nothing, and Settings shows it for any display faster than 48 Hz.
- The kernel always sends `IOMFBPARAM_ADAPTIVE_SYNC` 0 and exposes no `vrr_capable`.
- DCP says of this monitor over HDMI: `SupportsVariableRefreshRate` false, and every 2560x1440 timing has min and max VRR 0 (the attributes list 48-144 Hz).
- Asahi's PR 477 is open, marked NOUPSTREAM, behind a module parameter, needs a modeset to toggle, has requested changes, and is untested on HDMI.

Implementing it would be a kernel fork with black flashes, with no benefit here. The switch is gone; bring it back when the kernel reports `vrr_capable`.

## Stages, by benefit per risk

| # | Stage | Gain, measured | Risk | Blocker | Size |
|---|---|---|---|---|---|
| 1 | GPU copy to the scanout buffer | 2.0 ms to 0.45 ms per full frame, CPU part gone | low | none | S |
| 2 | Colour probe, then sRGB correct | every pixel on this monitor | low for the probe; **black screen** possible until booted | owner boot | S if DCP does it, M if shaders |
| 3 | Sample NV12 without copying | 0.34-0.66 ms per video frame | medium | none | S-M |
| 4 | Screen sharing over dmabuf | up to three full-frame CPU copies to none | low | none | M |
| 5 | Sample LINEAR dmabufs, then tiled and compressed | 0.56 ms per full window, 4 bytes/px per window, 20 % on client render | medium, **tearing** | stage 6 for Firefox-grade sync | M-L |
| 6 | Stop blocking on the GPU | compositor thread free during GPU work (0.5-8 ms a frame) | medium, **freeze** if a fence never signals | none | M |
| 7 | `wp_color_management_v1`, P3, 10-bit | Firefox and players show P3 correctly | low to medium | stage 2; Smithay has no support, the XML is in `wayland-protocols` | M-L |
| 8 | Windowed video on a plane | KWin's ~1 W on a laptop; not separable here | **high: firmware crash, black screen** | kernel plane change, stage 3 | M |
| 9 | HDR | PQ on HDR monitors | high | kernel project, stage 7 | L |

1. **Output copy.** On a real GPU `Bind<Dmabuf>` imports the dumb buffer as a Vulkan buffer and records one image-to-buffer copy; the mapping and memcpy stay only for lavapipe. A failed import falls back to the old path. The swapchain never writes a buffer being scanned out, so this adds no tearing. Chosen over rendering straight into the buffer: that saves only another 0.37 ms, renders slower into LINEAR, and blur needs a readable target.
2. **Colour.** First the owner step below. One kernel commit sets `BG_SRGB` for RGB surfaces on external displays, for one test boot. If DCP converts, keep it. If not, add the shader matrix from EDID primaries, which also serves displays DCP cannot help.
3. **NV12.** Import once per buffer, sample directly, keep the copy for the CPU renderer. Release a buffer only after the flip that replaced it. The media engine's pool must have enough buffers; today's copy hid that. Hardware decode on `fix-media` feeds this directly, and is what makes stage 8 worth measuring.
4. **Capture.** `ext-image-copy-capture` offers dmabufs; the compositor copies with the GPU; `cast.rs` negotiates `SPA_DATA_DmaBuf` and keeps MemFd as the fallback. No display risk, since only capture changes.
5. **Direct window textures.** The trap: sampling a client's buffer means the client must not write until release, and the compositor must wait for its fence. Without that, windows tear. Closing animations and stretch-on-resize must hold the buffer. Verify `linux-drm-syncobj-v1` on Apple first. Tiled buffers cannot use a plane, so feedback must stay per surface: LINEAR for what is on a plane, compressed for the rest, or the shell loses plane 2. Firefox then gets the same import path.
6. **Asynchronous frames.** Export a `sync_file`, wait in the event loop, record render time at fence signal. Stage 1 removes the CPU copy that today forces the wait. A fence that never signals must go through the existing GPU-loss recovery.
7. **Tagged content.** Per-surface description, conversion in shaders or DCP's per-surface fields; 10-bit composition removes banding under Glass. DCP lists no 10-bit RGB mode for this monitor at 2560x1440@120 (only 8-bit RGB, or 12-bit YCbCr), so at 120 Hz the link stays 8-bit and the gain there needs checking.

### Left for the owner's optimisation pass

- Frame clock: the max of 16 stays until there is a camera number. One 8 ms frame moves the latch about 5 ms earlier for 133 ms at 120 Hz; a decaying window may be better, but is a guess until measured.
- Glass: reuse a blur until its backdrop changes, smaller blur passes, the neighbour's redo. Measure it after stage 1, since it then has about 1.5 ms more room.
- Ten wake-ups per client update.
- One compositor image shared across the swapchain (saves up to about 45 MB at 1440p).
- Rendering straight into the scanout buffer when no blur is on screen.
- A cursor on an overlay: DCP has no cursor plane, and the 32x32 and off-screen rules make edges awkward.
- shm without a copy (`udmabuf`, as KWin does): the toolkit uses shm only for first frames.

### Not a problem

Text at 1x: the audit found it even and a little soft, not defective, and macOS also renders unhinted and greyscale (from memory). No change unless a side-by-side shows otherwise.

## Owner steps

1. **Is colour wrong today?** Toggle the monitor menu between its wide default and its sRGB mode. A clear jump in reds and skin tones means we are unmanaged. One minute.
2. **Test boot for stage 2**, with a kernel from main: look at the desktop and a known photo against the monitor's sRGB mode.
3. **Latency with a phone.** 240 fps slow motion, framing a key and the screen, in Terminal at 120 Hz. Ten presses; count frames from the key bottoming out to the glyph (each 4.2 ms). Add the same with a mouse button and a menu. The mouse and keyboard's own delay is included, so it compares only against the same run on a Mac. Ten minutes, before and after stage 6.
4. **After stage 1,** drag a large window in Glass on the real display; the frame numbers need the real path.

## Not measured

- No real client ran through a private compositor per type. The headless compositor does not exercise the scanout copy, so steps were timed alone.
- GPU clock was whatever firmware chose; other agents' small jobs ran alongside.
- Client-side copy, per-window memory in practice, DRAM bandwidth, hardware decode, windowed video on a plane, DCP's colourspace behaviour, input-to-photon latency.

## Sources

- [Asahi progress report 7.0](https://asahilinux.org/2026/04/progress-report-7-0/), [PR 477](https://github.com/AsahiLinux/linux/pull/477), [Mesa Asahi docs](https://docs.mesa3d.org/drivers/asahi.html)
- [KWin overlays, Phoronix](https://www.phoronix.com/news/Plasma-6.5-Overlay-Planes-KWin), [merge request](https://invent.kde.org/plasma/kwin/-/merge_requests/7795), [XDC 2025 slides](https://indico.freedesktop.org/event/10/contributions/424/attachments/275/363/XDC2025.pdf)
- [GNOME 48 colour management](https://www.phoronix.com/news/GNOME-wp_color_management_v1), [the protocol](https://wayland.app/protocols/color-management-v1)
- Apple: [CAMetalLayer](https://developer.apple.com/documentation/quartzcore/cametallayer.md), [edrMetadata](https://developer.apple.com/documentation/quartzcore/cametallayer/edrmetadata.md), [CAMetalDisplayLink](https://developer.apple.com/documentation/quartzcore/cametaldisplaylink.md), [IOSurface](https://developer.apple.com/documentation/iosurface)
