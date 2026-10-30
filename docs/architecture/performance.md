# Performance

Measured costs of the session, how to measure them, and what changed them. The budgets for
frame times are in [`../decisions/0001-rendering.md`](../decisions/0001-rendering.md) and are
checked by `ci/perf`; this page tracks memory, CPU per interaction and size.

## How to measure

- **Shell clients:** `unbound-footprint` (`crates/testing/bench`) starts each client on a
  headless compositor with a 2928×1818 output, lets it settle, optionally moves the pointer
  back and forth 40 times, and prints RSS + swap, Vulkan device memory (the
  `/memfd:allocation` mappings of lavapipe), threads and the client's CPU time per pointer
  move. Run it inside a private bus so that the live session is not touched:

  ```sh
  cargo build --release -p unbound-bench -p unbound-dock -p unbound-desktop-surface
  dbus-run-session -- target/release/unbound-footprint \
      --hover 1464,1790,1464,1500 target/release/unbound-dock \
      --hover 2864,70,2600,900 target/release/unbound-desktop-surface
  ```

  `--settle <seconds>` waits longer before measuring (40 s shows a hidden window after it has
  released its renderer).
- **Binary sizes:** `cargo build --profile dist -p <package>`; packages are built with this
  profile.
- **Crate counts:** `cargo tree -e normal -p <package> --prefix none | sort -u | wc -l`.

All numbers below are from the development VM: 10 cores, lavapipe (Mesa 25.0.7, Vulkan on the
CPU), so GPU memory shows up as process memory. On Apple Silicon GPU memory is also system
memory, so the device-memory savings carry over.

## 2026-10-30: atlas, shared device, release when hidden, damage

Before: `38999eb`. After: `desktop/efficiency`. Same headless setup, wallpapers and Desktop
folder.

| Client | RSS + swap before | after | Device memory before | after | Threads before | after |
| :--- | ---: | ---: | ---: | ---: | ---: | ---: |
| Dock | 183.8 MiB | 126.4 MiB | 67.5 MiB | 11.0 MiB | 32 | 31 |
| Menu bar | 189.3 MiB | 121.4 MiB | 65.1 MiB | 4.4 MiB | 34 | 34 |
| Desktop surface | 392.2 MiB | 365.5 MiB | 126.3 MiB | 106.3 MiB | 29 | 28 |
| Launcher, hidden 40 s | 135.1 MiB | 119.1 MiB | 0 | 0 | 29 | 7 |

- **Image atlas sized to the surface.** Vello's default atlas is 4096×4096 (64 MiB, created on
  the first image or colour glyph, up to eight of them). Each renderer now starts with an atlas
  the size of its surface rounded up to 256 px and grows it only when an image does not fit,
  keeping the same total budget. Dock −57 MiB, menu bar −68 MiB.
- **One device per process, released when hidden.** Windows share one Vulkan device. A window
  hidden for 30 s drops its renderer, frame textures, `wl_shm` buffers and, with the last
  window, the device and lavapipe's worker threads (launcher: 29 → 7 threads). Showing it again
  draws on the CPU until the GPU is back, as at start. The live session's launcher, which has
  been opened and so holds its atlas and swapchain, measured 235 MiB in the audit; it now drops
  all GPU state 30 s after it closes.
- **The first frame's CPU buffers are freed** once the GPU presents (desktop surface −27 MiB:
  a 2928×1818 frame and its `wl_shm` buffers).
- **Damage.** Each frame is diffed against the previous one item by item, descending into
  groups whose clip, offset, opacity or scale did not change. Only the changed regions are
  redrawn: on the GPU into a kept frame texture with the rest of the scene clipped away, on the
  CPU region by region, with only those regions damaged on `wl_shm` buffers. Hovering an icon on
  the desktop surface no longer redraws the wallpaper:

  | CPU per hover change (client process) | before | after |
  | :--- | ---: | ---: |
  | Desktop surface, icon ↔ empty desktop | 264.5 ms | 109–129 ms |
  | Dock, icon ↔ outside | 119.5 ms | 116–121 ms |

  The numbers include every animation frame of the hover (the hover fades, the Dock magnifies),
  and on lavapipe the "GPU" work is CPU time in the client. The Dock repaints its whole bar while
  it magnifies, so it gains little. On `wl_shm` the compositor now also uploads only the damaged
  rows; with Vulkan presentation wgpu still damages the whole surface.

What remains in the desktop surface's 365 MiB on lavapipe: device memory 106 MiB (atlas with
the wallpaper, the frame texture, the depth texture and the swapchain), lavapipe's presentation
buffers 62 MiB, libLLVM 51 MiB, decoded wallpapers and heap 94 MiB.

## 2026-10-30: one bar process

`unbound-bar` hosts the Dock and the menu bar of a display in one process: one Wayland connection
(the menu bar's surface opens the Dock's beside it with `Window::open_beside`, both driven by
`run_surfaces`), one connection per D-Bus bus, one font database and one Vulkan device. Same
headless setup as above, release build, the Dock and menu bar settled side by side:

| Client | RSS + swap | Device memory | Threads |
| :--- | ---: | ---: | ---: |
| Dock + menu bar before (two processes, table above) | 247.8 MiB | 15.4 MiB | 65 |
| `unbound-bar` (one process) | 160.9 MiB | 14.5 MiB | 37 |

−87 MiB (−35 %) and 28 fewer threads per display.

## 2026-10-30: dependency trims

| Package | Crates before | after | Change |
| :--- | ---: | ---: | :--- |
| unbound-search | 20 | 14 | `notify` replaced by `unbound-watch` (rustix inotify) |
| unbound-config | 109 | 101 | same, plus `tracing-subscriber` without `env-filter` |
| unbound-notify | 79 | 77 | `tracing-subscriber` without `env-filter` (regex) |
| unbound-dock | 267 | 259 | both |

`RUST_LOG` keeps working for levels and per-target directives (`warn,unbound_dock=debug`);
regex filters on span fields are gone.

Kept on purpose:

- `vello_cpu`'s `png` feature: it also enables PNG decoding of bitmap colour glyphs in `glifo`
  (shared with `vello_gpu`), which Noto Color Emoji (CBDT) needs. The `png` crate is linked
  through `image` anyway.
- `ab_glyph` as a dev-dependency: the render tests use it to compute reference coverage
  independently of `skrifa`, which the renderer uses.

## Binary sizes

Packages used the default release profile (opt-level 3, no LTO, 16 codegen units, unwinding;
cargo-deb strips). They now use `dist`: fat LTO, one codegen unit, `panic = "abort"`, stripped.
Nothing in the workspace catches panics (`catch_unwind`), and the workspace lints forbid
panicking in production code; with `abort` a panic in any thread ends the process, and systemd
or the session restarts it. Test and dev profiles are unchanged.

| Binary (stripped) | release (`38999eb`) | release (this branch) | dist | Change |
| :--- | ---: | ---: | ---: | ---: |
| unbound-dock | 19.87 MB | 19.94 MB | 14.63 MB | −26.6 % |
| unbound-config | 4.63 MB (installed) | 4.27 MB | 2.63 MB | −38.4 % |

Dock sections, release → dist: `.text` 12.03 → 9.09 MB, `.rodata` 4.19 → 3.94 MB,
`.eh_frame` 1.59 → 0.72 MB, `.gcc_except_table` 0.63 → 0.01 MB. Linking the Dock with fat LTO
took 2 min on the VM with two jobs; CI builds all packages in one `cargo build --profile dist`
so the links run in parallel, then runs `cargo deb --no-build` per package.

## Stale daemons

The four `unbound-config` processes and the portal from `session-6.scope` that the audit found
were not left by logout. `session-6.scope` is the shell the developers' tools run in; test runs
and measurements there use `dbus-run-session`, whose private bus D-Bus-activates the installed
services directly (no systemd on that bus). When the bus went away the services kept running,
because every daemon (`unbound-config`, `-notify`, `-power`, `-tray`, `-automount`,
`xdg-desktop-portal-unbound`) parked its main thread forever. They now serve until their session
bus connection closes (`unbound_bus::serve_until_closed`) and then exit. In a real session the
units are `PartOf=graphical-session.target` and are stopped with it, as before. Reproduced and
checked: a `dbus-run-session` that activates the old `unbound-config` leaves it behind with
PPID 1; the new one exits with the bus.

## ICU segmentation data

Parley's `complex-scripts` feature compiles ICU4X's dictionary and LSTM segmenters into every
binary that lays out text: about 2.9 MB of `.rodata` per binary (Thai, Lao, Khmer and Burmese
line and word breaking, and Chinese and Japanese word selection), 15 binaries.

| Option | Saving | Cost |
| :--- | ---: | :--- |
| Drop `complex-scripts` | 2.9 MB × 15 | Thai, Lao, Khmer and Burmese text no longer wraps between words (lines break only at spaces), double-click selects whole runs of CJK text. Unacceptable for a desktop that shows arbitrary file names and notifications. |
| Enable it only in some packages | 2.9 MB per package left out | Any package that shows user text needs it (Files, notifications, launcher, Settings, terminal, dock tooltips), so little is saved. |
| Load the data at run time from one ICU4X blob in `/usr/share/unbound/icu/` | 2.9 MB × 14, one shared page-cache copy | Needs a data-provider hook in Parley (`parley_engine` calls the compiled-data constructors directly); an upstream change. |
| One multicall binary for the GUI programs | the whole shared stack, about 15 MB × 14 | Packaging work; also fixes this. |

Recommendation: keep the feature, propose the data-provider hook upstream to Linebender, and
get the bigger saving through the one-process shell or a multicall binary. Not implemented.
