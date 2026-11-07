# Media playback

## What the user gets
Double-click a song or film: Bande Player opens and plays it, as QuickTime does: one window, controls
that fade over the picture, Space, arrows, full screen, speed, audio and subtitle tracks. `Space` in
Files plays it in Quick Look. Media keys and the Control Center drive it through MPRIS, and a playing
video keeps the display awake. A codec that crashes takes down neither Files nor the window.

## Design
- **Engine process.** `bande-media-engine` (apps repo, `apps/player`, the one thing linking GStreamer)
  demuxes, decodes, keeps A/V sync and plays sound. The app and Quick Look talk to it over a
  `SOCK_SEQPACKET` pair with `bande-media` (desktop repo: protocol and client, no GStreamer). The
  desktop neither links nor builds GStreamer; Files shows the icon when the engine is not installed.
- **GStreamer, not libav directly.** It is the only Debian path to the V4L2 stateless decoder of the
  M2 (Debian's FFmpeg has no v4l2-request), and picks hardware first and software when that fails, per
  stream. Decoders underneath are Debian's FFmpeg (`gstreamer1.0-libav`) and dav1d
  (`gstreamer1.0-dav1d`). Our own parsers and decoders (the old web plan) would be years of work.
- **Pixels.** The engine sends each buffer's fd once and then only its id; the app attaches it to an
  `ExternalSurface` (a desync subsurface), so a frame wakes neither the scene nor the window's
  renderer, and returns the id on `wl_buffer.release`.
- **Packages.** `gstreamer1.0-plugins-bad` pulls GTK 3 and SDL (+156 MB): never a dependency. Hardware
  decode gets a small package of its `v4l2codecs` and `videoparsersbad` plugins, Apple only.

## Stages
1. Playback: engine, Player, MPRIS, media keys, keep awake; frames as XRGB in shared memory.
2. Quick Look in Files.
3. Efficient frames: NV12 dmabufs (`udmabuf` for software decode) sampled by the compositor without a
   copy, with `color-representation-v1`.
4. Hardware decode: AVD driver in the kernel build, `avd-fw` packaged, the plugin package. Browser
   decode needs VA-API or a Firefox V4L2-request path and is separate.
