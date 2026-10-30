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
- **Session frame times:** `unbound-session-bench` runs the compositor in-process on a
  headless 2928×1818 output at 1.5×, starts the desktop surface, the bar, the launcher, Files
  (on a folder of 400 files) and the terminal from its own directory, and replays typical
  actions one refresh apart: the pointer moving over Files, dragging Files by its title bar,
  typing 60 characters into the terminal, scrolling the file list, opening and closing the
  launcher, the overview and a context menu of Files twice, and showing two notification
  banners that expire after 1.5 s (it starts `unbound-notify` and `unbound-notifications` for
  this and sends them with `gdbus`). For each it prints the compositor's frame time (p50, p99),
  its CPU time per frame (including lavapipe's threads), the clients' CPU time per frame, the
  cores kept busy and the pixels repainted per frame. Arguments filter the actions by name;
  `--frames` lists every frame, `--full-effects` keeps blur on a CPU renderer, `--snapshots
  <dir>` writes the screen after each action as PPM. Run it inside the test bus:

  ```sh
  cargo build --release -p unbound-bench -p unbound-desktop-surface -p unbound-bar \
      -p unbound-launcher -p unbound-files -p unbound-term -p unbound-notify \
      -p unbound-notifications
  dbus-run-session --config-file=crates/testing/test-bus/data/session.conf -- \
      env -u WAYLAND_DISPLAY target/release/unbound-session-bench
  ```

  Other developers' builds share the VM's ten cores, so wall-clock frame times move with the
  load average the tool prints; compare runs taken back to back, and the CPU columns, which
  move less.
- **Idle wake-ups:** sum the `voluntary_ctxt_switches` and `nonvoluntary_ctxt_switches` of every
  thread in `/proc/<pid>/task/*/status` twice, some seconds apart, and divide the difference by
  the interval; the per-thread `comm` names show which thread wakes. Run the services inside the
  test bus and the bar and Control Center under `unbound-footprint --settle <seconds>` (its
  headless compositor), with `XDG_RUNTIME_DIR` pointing at a private directory that links
  `pipewire-0` so that PipeWire is real but no client reaches the live session's display.
- **Binary sizes:** `cargo build --profile dist -p <package>`; packages are built with this
  profile.
- **Crate counts:** `cargo tree -e normal -p <package> --prefix none | sort -u | wc -l`.

All numbers below are from the development VM: 10 cores, lavapipe (Mesa 25.0.7, Vulkan on the
CPU), so GPU memory shows up as process memory. On Apple Silicon GPU memory is also system
memory, so the device-memory savings carry over.

## 2026-10-30: menus, launcher and banners in motion

Frames per action with `unbound-session-bench launcher menu notification`, `main` at `bef5145`
against `desktop/motion-polish`, run back to back twice, load average 4 to 5 (lavapipe, CPU
clients). "Frames" counts every composed frame of the action, which runs twice.

| Action (twice) | Frames before | after | Frame p50 before | after | Repainted Mpx per frame before | after |
| :--- | ---: | ---: | ---: | ---: | ---: | ---: |
| Opening and closing the launcher | 8 | 16 | 14–17 ms | 16–18 ms | 3.99 | 3.9–4.1 |
| Opening and closing a context menu | 6–7 | 43–51 | 10–11 ms | 1.7–2.6 ms | 3.0–3.6 | 0.64–0.72 |
| Two notification banners in and out | 237–243 | 155–161 | 0.7–0.8 ms | 0.9 ms | 0.14 | 0.18 |

- **The launcher is as big as its card.** Its layer surface is the card plus 48 pt for the
  shadow (816 × 617 pt instead of the whole display), anchored to the top edge with the card's
  old position as margin. The dimmed backdrop is drawn by the compositor:
  `unbound_shell_v1.set_scrim` (version 15) dims and, on a GPU, blurs the display behind a
  mapped layer surface, and a press on the scrim reaches nothing below it and is sent to the
  surface's client as `scrim_pressed`, which closes the launcher. Each launcher frame now
  uploads 4.5 MB instead of 21.3 MB and shades 13.3 Mpx instead of 17.5. Changing the dimming
  still repaints the display, so opening and closing keep a full repaint; an undimmed launcher
  would not.
- **The launcher's appear animation ran for one frame.** It asked for the next frame from
  inside `scene`, which drawing then cleared, so the fade jumped from its first frame to the
  end; it now answers `App::animating`, which is why it draws twice as many frames. The
  scrim's opacity follows the card's fade (`fade`), the card rises 8 pt with `quick`.
- **Menus fade and slide in, and fade out.** A menu and each submenu appear with a `fade` of
  their opacity and a 3 pt `quick` slide away from their anchor (fade only with Reduce
  Motion), driven by the popup's frame callbacks. The menu is rendered once; each animation
  frame only shifts and fades those pixels, about 0.2 Mpx a frame. Closing hands the picture to
  a popup without a grab, so the keyboard and pointer are free from the first frame of the
  fade, and a key typed during the chosen item's blink reaches the window. The one frame per
  open and close that repaints the whole display (3 to 5 Mpx, before and after) is not the
  menu's; its cause is still to be found in the compositor.
- **Banners stop when the motion is invisible.** Notification banner and Notification Center
  positions, the Dock's reorder shift and its label now settle at 0.05 pt instead of 1/512 of
  a point: 240 → 158 frames for two banners. The compositor's zoom settles at a quarter of a
  physical pixel of the largest display and the accessibility flash at 1/512 of opacity.

## 2026-10-30: idle wake-ups to zero

An idle desktop should not wake the CPU. Wake-ups per second of each process while nothing
happens, before (`main` at `0752344`) and after (`desktop/idle-zero`), measured as described
above over 10 s before and 30 s after, debug builds, the bar on the real system bus
(NetworkManager, UPower and BlueZ running) and PipeWire:

| Process | before | after | What woke it |
| :--- | ---: | ---: | :--- |
| unbound-bar | 23.7 (live session: 20.0) | 0 | status polling, the clock, the `blocking` pool |
| unbound-controlcenter | 2.0 | 0.1 | the `blocking` pool; the 0.1 left is three one-off wake-ups of its main thread, not periodic |
| unbound-sound | 2.0 | 0 | the `blocking` pool |
| unbound-notify | 2.0 | 0 | the `blocking` pool |
| unbound-config | 2.0 | 0 | the `blocking` pool |
| unbound-power | 2.0 (live session) | 0 | the `blocking` pool |
| unbound-tray | 2.0 | 0 | the `blocking` pool |
| unbound-clipboard | 2.0 (live session) | — | the `blocking` pool |

The clipboard daemon needs the privileged data-control global, which only the real compositor
grants, so it was measured in the live session only (read-only, from `/proc`); its one waker was
the `blocking` pool, which it shares with every other process, so it drops to 0 with them. The
compositor (2.1 in the live session, all from the same pool) likewise.

- **The `blocking` thread pool.** zbus connects its Unix sockets through `blocking::unblock`, so
  every process that talks D-Bus has a `blocking-1` thread. In `blocking` 1.7.0 an idle pool
  thread exits only when the pool is over its thread limit (500) — never — and otherwise wakes
  every 500 ms forever (upstream smol-rs/blocking#81 and #83, open). 1.6.2 lets an idle thread
  exit after 500 ms, as documented. The workspace pins `blocking = "=1.6.2"` until a fixed
  release; the Portal and the polkit agent now take it from the workspace instead of their own
  `=1.7.0` pins. Nothing else in zbus or `async-io` wakes an idle process: zbus's executor and
  the `async-io` reactor block in `epoll` without a timeout.
- **Battery, Wi-Fi and Bluetooth by signal.** The bar re-read UPower, NetworkManager and BlueZ
  every 3 s (dozens of D-Bus calls each time, which also woke zbus's reader and the `async-io`
  thread). `unbound_status::devices::follow_devices` now subscribes to every signal of the three
  services and to their owner changes, and re-reads only the service that spoke, coalescing
  bursts (a scan that changes twenty access points is one read). Changes show at once instead of
  up to 3 s late; a service that starts or restarts is picked up from its `NameOwnerChanged`.
- **Sound by PipeWire's events.** The bar polled the audio snapshot every 200 ms. The PipeWire
  thread already updates a shared state from registry, node and route events and signals a
  condition variable; `Audio::changed_since` blocks on it, and the bar's sound controls run on
  their own thread. Waiting for a command's reply blocks on its channel instead of checking the
  connection every 250 ms; a lost connection drops the pending replies, which ends the wait.
- **The clock by the minute.** The bar checked its clock every second. A `timerfd` on
  `CLOCK_REALTIME`, armed for the next whole minute with `TFD_TIMER_CANCEL_ON_SET`, now wakes it
  once a minute, at the minute, and at once when the clock is set or the machine resumes, so the
  time is right after suspend even though the bar's own timers do not run during it.
- **Status notifier items by narrow match rules.** The tray host subscribed to every signal on
  the session bus; it now asks the bus only for status-item and watcher signals, the watcher's
  owner changes and the app-menu `LayoutUpdated`, so other apps' signals no longer wake the bar.

## 2026-10-30: software rendering (lavapipe at 1.5×)

The development VM's GPU is virtio-gpu without 3D, so the compositor and every toolkit client
rendered through lavapipe on the CPU, at 1.5× on a 2928×1818 output (5.3 Mpx a frame). Real
hardware renders on its GPU; VMs and machines without a usable GPU must still feel smooth.
Measured with `unbound-session-bench`, two rounds each, back to back, load average 6.5 to 8.2
on ten cores (`main` at `c2aa894` with the idle fix below, then this branch):

| Action | Frame p50 before | after | Frame p99 before | after | Compositor CPU per frame before | after | Clients CPU per frame before | after |
| :--- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Pointer over a window | 0.8 ms | 0.7–1.0 ms | 5–85 ms | 2–7 ms | 4–8 ms | 4–5 ms | 0–2 ms | 0.2–0.5 ms |
| Dragging a window | 22–25 ms | 5.5–5.8 ms | 36–41 ms | 19–33 ms | 119–140 ms | 30–37 ms | 0.1 ms | 0 ms |
| Typing in the terminal | 13–18 ms | 0.7–0.8 ms | 52–74 ms | 17–31 ms | 72–79 ms | 5–6 ms | 38–51 ms | 1.2–1.4 ms |
| Scrolling a list | 24–31 ms | 1.8–1.9 ms | 54–74 ms | 9–13 ms | 131–158 ms | 9–10 ms | 56–81 ms | 5 ms |
| Opening the launcher | 33–34 ms | 1–4 ms | 74–118 ms | 142–225 ms | 145–177 ms | 101–130 ms | 54–77 ms | 18–30 ms |
| Opening the overview | 33–50 ms | 12.4–12.5 ms | 62–118 ms | 30–42 ms | 180–235 ms | 64–67 ms | 3–11 ms | 0.2–0.3 ms |

While dragging, the session kept 3.2–3.5 cores busy before and 0.9–1.2 after; while
scrolling, 3.2–3.4 before and 0.6 after. Pixels repainted per frame: typing 0.57 → 0.05 Mpx,
scrolling 1.58 → 0.20 Mpx.

- **Idle clients no longer spin.** Every toolkit client polled its appearance and
  accessibility wake-up sockets; when the thread writing to one ended (no settings portal or
  no accessibility bus, as on the test bus, or a portal that crashed) the socket hung up and
  `poll` returned at once forever: each client used a full core while idle. The windows now
  keep the writing ends open. The "before" column above already includes this fix, since
  without it the measurement is meaningless (five idle clients used 4.3 cores).
- **Opaque regions.** No toolkit window declared an opaque region, so beneath every window
  the compositor drew the wallpaper, three shadow layers (their shader evaluates a Gaussian
  integral per pixel) and any window below: five layers for each pixel of a moving window. A
  window whose background and frame are opaque now declares its window rectangle (the
  compositor already removes the rounded corners), and a surface without a frame its whole
  area when its background is opaque. Dragging: 119–140 → 31–39 ms of compositor CPU per
  frame on its own. `a_change_under_an_opaque_window_shades_only_the_window_on_top` checks it
  (5 layers shaded before, 1 after).
- **Clients draw on the CPU when the only Vulkan device is a CPU.** A window whose adapter is
  `DeviceType::Cpu` or named llvmpipe or lavapipe stays on `vello_cpu` and presents `wl_shm`
  buffers with only the changed regions damaged, instead of rendering through lavapipe and
  presenting the whole surface through Vulkan's WSI. The probe runs once per process, on the
  thread that used to start the GPU; later windows skip it. `UNBOUND_RENDERER=gpu` or `=cpu`
  overrides the choice (the toolkit's GPU tests and `unbound-toolkit-bench` set `gpu`). This is
  what took typing and scrolling below 2 ms. Clients on the CPU path start no lavapipe
  threads either.
- **Reduced effects on a CPU renderer.** `unbound-render` reports a CPU device
  (`DeviceInfo::software`) and then reduces effects: a blur region becomes a flat scrim of the
  sheet colour at 72 %, which does not depend on what lies beneath it, so a change under it
  repaints only the change (`with_reduced_effects_a_blur_region_is_a_flat_scrim…`). The bar is
  opaque since its Datum restyle, so this benchmark does not show it; it applies to the
  launcher, the notifications and Control Center, which still blur. The overview's own
  full-screen blur, merged after these measurements, is left out on a CPU renderer and its
  scrim stays, so a 5.3 Mpx blur does not run on every frame of its animation. Shadows stay: with opaque regions only
  their rim outside the window is shaded, which is not where the time goes.
- **Cursor.** Moving the pointer re-renders nothing: the cursor element is `Kind::Cursor` in a
  memory buffer, which Smithay's `DrmCompositor` puts on the cursor plane that virtio-gpu
  offers (64×64; the cursor is 36 px at 1.5×), and on the primary plane damage covers only
  the two cursor rectangles. "Pointer over a window" above is the hover feedback of Files.
- **Threads.** lavapipe starts one rasterizer thread per core. Capping `LP_NUM_THREADS` for the
  compositor saved CPU but made frames slower (three rounds, dragging p50: 2 threads 12–16 ms,
  4 threads 7–16 ms, 6 threads 5–9 ms, default 5–6 ms; overview p50: 22–40, 12–35, 10–11,
  9–17 ms), so it is left at its default. Clients on the CPU path have no lavapipe threads.
- **A pure CPU compositor (pixman) was not built.** After the changes above, dragging a
  1578×1011 px window repaints 1.6 Mpx a frame for about 33 ms of CPU spread over lavapipe's
  threads (5.5 ms of wall time). A pixman path could compose plain textures for less CPU on
  one core, but it would need a second implementation of every element the compositor draws
  (rounded clips, shadows, scrims, the scaled and faded windows of the overview, minimise and
  workspace animations) and Smithay's pixman renderer cannot draw them. Not worth it now.

What remains VM-bound:

- **Opening the launcher**: the launcher was a full-screen translucent surface, so its first
  frame uploaded 21 MB and shaded 12 Mpx. It is now the size of its card with the dimming drawn
  by the compositor (see "menus, launcher and banners in motion" above).
- **The overview** moves every window every frame (3 Mpx a frame): 12 ms p50, 30–42 ms p99.
- **Dragging** repaints the window at its old and new place each frame (1.6 Mpx): 5–6 ms p50
  when cores are free, more when other builds hold them.

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
