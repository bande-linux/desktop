# Quality review: platform, toolkit and test-wayland

> Since 2026-11-01 the desktop keeps only the few tests in [`tests.md`](tests.md) and no golden images. The test files, goldens and test helpers this review cites are gone, and its items about them no longer apply.

Scope: `crates/platform/{compositor,render,protocols}`, `crates/toolkit/{design,graphics,widgets,platform,toolkit}` and `crates/testing/test-wayland`, at `main` 2d497b1 (2026-10-31).
That is about 70,500 lines of Rust in 230 files.

This was a read-only review: no cargo runs, because the build slots were full.
Evidence comes from reading the code, a token-level scan for unused `pub` items across the whole workspace, a scan for functions defined more than once, and `strings` on the installed `/usr/bin/bande-compositor`.
Line savings are counted from the source.
Performance wins are estimates; each perf task names the bench that should confirm it (`crates/testing/bench`: `bande-footprint`, `bande-session-bench`, `bande-toolkit-bench`, `tests/headless/frames.rs`).

What is already clean:
- no code comments; the only `///` is a required `# Safety` section at `graphics/src/gpu.rs:56`;
- no `unwrap`/`expect`/`panic!` in production code (the workspace lints enforce it);
- every `#[cfg(test)]` module is real.

The debt sits in four areas:
1. hot-path allocation and duplicated work in the compositor;
2. version-history rot in the private protocol;
3. dead or duplicated client code in the toolkit;
4. a few god modules.

Each task below is sized for one agent in at most 1 to 2 hours, unless it says otherwise.

---

## Ranked fix tasks

### 1. Hit-test once per pointer event, without rebuilding the scene (compositor)
- **Evidence:**
  - `Headless::move_pointer_to` (`compositor/src/input.rs:512-553`) calls `surface_under` (`input.rs:169`), which goes through `pointer_target` and does a full `State::scene(false, screen)` build (`frame.rs:172-184`).
  - It then calls `point_at_frame` (`frame.rs:193`), which calls `frame_under`, which calls `pointer_target`: a second full scene build.
  - `drag_over` and the button path (`input.rs:571-583`) can add a third and a fourth.
  - `scene()` (`state.rs:1197-1281`) allocates about 20 vectors per call:
    - four `layers()` collects, each taking a layer-map mutex;
    - one `popup_surfaces` Vec per window;
    - `visible`;
    - three `partition`s, two Vecs each;
    - four `window_entries`;
    - the closing and overview Vecs;
    - the final `concat`.
  - `stack()` (`state.rs:1183`) wraps the same build and runs every frame in `latch_screen` (`state.rs:522`).
- **Change:**
  - Compute `pointer_target(location)` once in `move_pointer_to` and `press_button`, and pass the `Target` into `point_at_frame`, `drag_over` and `refocus_pointer`. `point_at_frame` takes `Option<(WlSurface, Zone)>` instead of re-querying.
  - Then make `scene()` fill one reusable `Vec<Placed>`: iterate the windows three times with filters instead of partitioning, and push layers in place.
- **Win:**
  - Halves hit-testing cost at once.
  - With the allocation-free `scene()`: about 40,000 fewer allocations/s and about 8,000 fewer mutex locks/s at a 1000 Hz pointer.
  - Measure with a criterion-free loop in `bande-session-bench`: 10,000 `move_pointer_to` calls over 12 windows.
- **Lines:** about −20.
- **Risk:** low. Ordering semantics are unchanged; `tests/headless/{input,window,popup,dnd}.rs` cover it.

### 2. One text and label renderer for every compositor overlay
- **Evidence:** the compositor creates up to five independent `TextSystem` + `CpuRenderer` pairs. Each holds its own fontique collection, source cache and glyph caches.
  - `Decorations`: `frame.rs:80-81`, always created.
  - `Alerts`: `alert.rs:46`, `RefCell<Labels>`, always created.
  - Input-source HUD: `keymap.rs:71`, lazy.
  - Overview: `overview.rs:75`, lazy.
  - Switcher: `switcher.rs:218`, lazy.
- **Evidence, duplicated paint code:**
  - `Decorations::paint` (`frame.rs:381-436`) repeats `Labels::paint` (`label.rs:114-135`): allocate RGBA, `render_into`, swap R/B per pixel, upload.
  - The same swizzle exists a third time in the toolkit (`toolkit/platform/src/window.rs:1943-1952`, `1986`).
- **Change:**
  - Put a single `labels: Labels` on `State` and let `Decorations` paint through `Labels::paint`.
  - Drop the four other fields and the `get_or_insert_with(Labels::default)` calls.
  - Add `Fourcc::Abgr8888` to `TEXTURE_FOURCCS` (`render/src/lib.rs:39`; the format mapping at `lib.rs:386` already supports it) and upload CPU-rendered RGBA as `Abgr8888`. This deletes the per-pixel swap loops at `label.rs:124-126` and `frame.rs:426-428`.
- **Win:**
  - Estimated 4 to 12 MB less RSS: four fewer font collections and glyph caches.
  - No repeated fontconfig scan on the first Alt+Tab, overview or HUD.
  - The swap loop is a full per-pixel pass on every title and label repaint (230,400 pixels for a 1440 pt window title at 2×); it goes away.
  - Measure with `bande-footprint`.
- **Lines:** about −60.
- **Risk:** low. Goldens in `tests/headless/{switcher,decoration}.rs` catch channel mistakes.

### 3. Reset the private protocols to version 1 and delete 49 version gates
- **Evidence:**
  - `bande-shell-v1.xml:3` is at `version="17"` and `bande_app_v1` (`:463`) at `version="5"`, with 51 `since=` attributes.
  - The compositor and its clients are always built and shipped from one tree, and no backward compatibility is kept. Even so, the code checks versions in 49 places:
    - compositor: `shell.rs` (18), `apps.rs` (4), `workspace.rs` (3), `watchdog.rs` (1);
    - toolkit: `toolkit/platform/src/shell.rs` (23), plus the `shell_since` helper at `shell.rs:44-51`.
  - Binds hard-code versions too: `state.rs:307` (`17`), `toolkit/platform/src/window.rs:2387-2390` (`version.min(17)`, `min(5)`) and `test-wayland/src/shell.rs:176,284-295` (1, 2, 3).
- **Change:**
  - Set both interfaces to version 1, strip every `since`, bind at version 1 and delete every `*_SINCE` check.
  - Replace the 18 copies of `for shell in &self.shell_resources { if shell.version() >= X { shell.event(..) } }` with one `fn each_shell(&self, f: impl Fn(&BandeShellV1))`.
  - `shell_since` becomes `fn shell(&self) -> Result<&BandeShellV1, WindowError>`.
- **Lines:** about −150 Rust, −51 XML attributes.
- **Risk:** low. It is mechanical; every shell and app crate rebuilds against the new XML in the same merge.

### 4. Stop deep-cloning the scene twice per toolkit frame
- **Evidence:**
  - `Window::draw` stores `self.shown_scene = Some((scene.clone(), scale))` (`toolkit/platform/src/window.rs:1902`) to diff against the next frame.
  - For windows with chrome, `Chrome::frame_scene` first copies the whole content tree (`chrome.rs:376-380`, `content.items.clone()`).
  - `framed` clones again when there is no chrome (`window.rs:1657`).
  - The callers already own a fresh `Scene` (`run.rs:109-110`, `370-373`).
  - `Item` trees are nested `Vec<Item>`; only paragraphs are `Arc`.
- **Change:**
  - `Window::draw(&mut self, scene: Scene)`.
  - `frame_scene(.., content: Scene)` moves the items into the frame.
  - After rendering by reference, move the drawn scene into `shown_scene`.
  - Also update `framed_scene` (public, `chrome.rs`) and `Window::draw` callers in `testing/sample`.
- **Win:** two whole-tree clones per frame go away: for a Files window with 500 visible items, roughly 1,000 allocations per frame. Measure with `bande-toolkit-bench`.
- **Lines:** about −5.
- **Risk:** low.

### 5. Keep wgpu and vello_gpu out of the compositor binary
- **Evidence:**
  - The compositor uses only CPU and text items from `bande-graphics`: `CpuRenderer`, `TextSystem`, `Palette`, `TitleBar`, `pixels`.
  - Even so, `wgpu`, `vello_gpu`, `pollster` and wgpu's `naga` are compiled and linked into it (`graphics/Cargo.toml` lists them unconditionally).
  - `strings /usr/bin/bande-compositor` (19.0 MB) shows 33 `wgpu` and 2 `vello_gpu` source paths.
- **Change:**
  - Add a `gpu` feature to `bande-graphics` that gates `gpu.rs`, `GpuDevice`/`GpuRenderer`, `wgpu`, `vello_gpu` and `pollster`.
  - `bande-toolkit-platform` enables it; the compositor and `shell/screenshot` (CPU only) do not.
- **Win:**
  - Smaller compositor binary, estimated −2 to −5 MB before strip and LTO; to be measured.
  - `-p bande-compositor` builds skip wgpu and naga, so CI and Debian per-package builds get faster.
- **Lines:** about +10 (cfg attributes).
- **Risk:** low.

### 6. Purge dead code and unused dependencies
All the items below were checked by a whole-workspace search: each has zero callers outside its definition, or only callers in its own file.

**Unused, delete:**

| Item | Evidence | Lines |
| --- | --- | --- |
| `Window::capture_screen` and its whole `ext-image-copy-capture` client | `toolkit/platform/src/capture.rs:46-173`; registry binds at `window.rs:2392-2397`. The screenshot app has its own full client in `shell/screenshot/src/engine.rs`. | −170 |
| `Window::set_background_blur` and the whole `BackgroundEffect` client | `toolkit/platform/src/effect.rs`, 48 lines; `window.rs:66,345,2368-2370`. No app requests blur; the compositor keeps serving the standard protocol for third-party clients. | −55 |
| `set_corner_radius` | Toolkit wrapper `toolkit/platform/src/shell.rs:181`. Private protocol request `SetCornerRadius`: compositor `shell.rs:515`, `effect.rs:58-66`. No Bande client calls it, and third-party clients cannot reach the private protocol. | −40 |
| `window_workspace` event | `bande-shell-v1.xml:224-232`. Sent at `compositor/src/workspace.rs:690,720`; only `test-wayland/src/shell.rs:605` reads it. | −35 |
| `Shape::bands` | `graphics/src/scene.rs:70-99` | −30 |
| `Window::gpu_ready`, `Window::toplevel_state` | `toolkit/platform/src/window.rs:965,764` | −8 |
| `Toggle::is_on` | `widgets/src/toggle.rs:33` | −3 |
| `Headless::output_config`, `usable_area_on`, `set_main_output`, `output_scale` | `compositor/src/displays.rs:499,503,558`, `state.rs:578` | −16 |
| `pollster` dependency of `bande-toolkit-platform` | `toolkit/platform/Cargo.toml:11`; never imported | −1 |
| `accesskit` listed twice in `bande-toolkit` | `toolkit/toolkit/Cargo.toml`: `[dependencies]` and `[dev-dependencies]` | −1 |

**`pub` but used only in their own file, make private:**
- `widgets/src/menu.rs`: `SUBMENU_DELAY` (`:26`), `at_least_as_wide_as` (`:358`), `fit_height` (`:365`), `scroll_by` (`:434`), `open_submenus_left` (`:455`);
- `widgets/src/checkbox.rs:59` `set_state`;
- `toolkit/platform/src/window.rs`: `toggle_full_screen` (`:772`), `start_move` (`:899`), `start_resize` (`:905`);
- `graphics/src/gpu.rs`: `offscreen` (`:44`), `atlas_size` (`:229`);
- `compositor/src/clock.rs:60` `predicted_render`;
- compositor re-exports `DEFAULT_CURSOR_THEME`, `DESKTOP_NAME`, `session_environment` (`lib.rs:62-70`).

**Afterwards:** `ScreenImage` duplicates `bande_graphics::Snapshot` (`width`, `height`, `rgba`). Once the capture client is gone, the screenshot app should use `Snapshot`, or own `ScreenImage` itself.

- **Lines:** about −360.
- **Risk:** low. Check with the owner before removing `set_corner_radius`, in case the shell plans to adopt it.

### 7. Take blocking file I/O off the compositor thread in the app switcher
- **Evidence:**
  - `Switcher::app` (`compositor/src/switcher.rs:242-281`) parses every `.desktop` file (`applications(&data_dirs(), ..)`) synchronously on the compositor thread. This happens on the first Alt+Tab and again every `ENTRIES_FRESH_FOR`.
  - `Switcher::image` (`:283-291`) finds and rasterises SVG icons with resvg on the same thread.
- **Change:**
  - Load entries and icons on a worker thread through a calloop channel; the overview and HUD pattern in `main.rs` shows how.
  - Pre-warm at session start.
  - Show the placeholder tile (`switcher.rs:351`) until the icon arrives.
- **Win:** removes an estimated 10 to 60 ms stall (hundreds of `.desktop` files plus several SVG rasterisations) from the first Alt+Tab after login and after each cache expiry. Measure with `tracing` spans around `running_apps`.
- **Lines:** about +30.
- **Risk:** medium (asynchronous state).

### 8. Split `compositor/src/state.rs` and name the core after what it is
- **Evidence:**
  - `state.rs` has 1,794 lines.
  - `State` has about 110 fields (`state.rs:143-245`), 20 of them smithay protocol globals that are touched only through their handlers.
  - The file also holds about 420 lines of handler `impl`s (`state.rs:1378-1794`: compositor, xdg, seat, layer shell, activation, fractional scale and others).
  - The core type is called `Headless` but also drives real DRM hardware (`main.rs:253`, `Headless::on_device`).
  - About 40 `pub` methods on `Headless` exist only for tests: `workspace_count`, `switcher_tiles`, `popup_rects` and others; see the scan list in `displays.rs`, `workspace.rs`, `session.rs`, `popup.rs` and `clock.rs`.
- **Proposed split (behaviour unchanged):**

  | New home | Contents |
  | --- | --- |
  | `state/globals.rs` | `struct Globals` holding every `*_state` and `_global` field |
  | `state/handlers.rs` | all smithay `*Handler` impls and `delegate_*` |
  | `state/compose.rs` | `compose`, `compose_at`, `compose_mirror`, `render_screen`, `relocated` (`state.rs:599-806`) |
  | `state/focus.rs` | `focus`, `frontmost*`, `refocus_after_losing`, `place_behind_frontmost` (`state.rs:986-1134`) |
  | `state/mod.rs` | `State`, `Headless`/`Compositor`, construction |
  | `probe.rs` | every test-only `impl Headless` accessor, gathered in one place |

- **Also:** rename `Headless` to `Compositor`. This is a mechanical rename over 457 references in 103 files; do it in a quiet window.
- **Lines:** about 0 (moves); `state.rs` drops to under 500 lines.
- **Risk:** low (moves only); medium for the merge if others touch `state.rs` that night.

### 9. Shared geometry helpers instead of three local copies
- **Evidence:**
  - An identical `fn contains(rect: Rect, point: Point) -> bool` exists in `widgets/src/lib.rs:343`, `toolkit/platform/src/chrome.rs:212` and `graphics/src/title_bar.rs:76`, plus inline copies at `compositor/src/frame.rs:96,147-151`.
  - The resize-edge classification (corner reach, then a −1/0/1 grid, then `ResizeEdge`) is written twice: `compositor/src/frame.rs:111-135` and `toolkit/platform/src/chrome.rs:180-204`.
  - The compositor's own `state::Rect { location, size }` (`state.rs:1137`) is smithay's `Rectangle<i32, Logical>` under another name. It is used in 65 places, and `tests/headless/popup.rs:514` and `toolkit/platform/tests/input.rs:1227` re-implement `contains` for it.
- **Change:**
  - Add `Rect::contains(self, Point) -> bool` to `bande-graphics` (`scene.rs`).
  - Add `pub fn resize_edge(window: Rect, point: Point, corner: f32) -> Option<Edge>` with a protocol-neutral `Edge` to `title_bar.rs`, next to `RESIZE_CORNER`. The compositor and the toolkit map `Edge` to their `ResizeEdge` and cursor.
  - Replace `state::Rect` with `Rectangle<i32, Logical>`, which gives `contains`, `overlaps` and `intersection` for free.
- **Lines:** about −90.
- **Risk:** low. The goldens and `tests/headless/decoration.rs` cover the resize zones.

### 10. Stop blocking on the GPU at the end of every compositor frame
- **Evidence:**
  - `VulkanFrame::finish_frame` (`render/src/frame.rs:441-446`) ends with `submit_and_wait()` (`render/src/lib.rs:1120-1131`): submit, then `wait_for_fences(u64::MAX)` on a single command buffer and fence.
  - `finish()` then returns `SyncPoint::signaled()` (`frame.rs:661-664`).
  - So the compositor thread sleeps for the full GPU time of every frame, blur passes included, and cannot read input or client requests during that time. The CPU and GPU never overlap.
- **Change:**
  - Use two command buffers and two fences in flight, and wait for the previous use of a buffer at `begin` instead of at `finish`.
  - Implement smithay's `Fence` for the Vulkan fence (exported as a `sync_file` through `VK_KHR_external_fence_fd`) and return it from `finish()`, so `DrmCompositor` waits or uses an in-fence.
  - Keep the synchronous path only when `exported` readbacks are recorded (`frame.rs:424-440`) and in `copy_target_to` (`lib.rs:1263`).
- **Win:**
  - The compositor thread is freed for the GPU time per frame (estimated 2 to 6 ms on AGX at 2560×1600 with blur).
  - Lower input latency under load.
  - Measure with `tests/headless/frames.rs` and `bande-session-bench` frame stats.
- **Lines:** about +60.
- **Risk:** high (resource lifetime against the graveyard, staging reuse). Budget 2 hours; split off the fence export if it runs over.

### 11. One portal-settings follower and one D-Bus connection per bus
- **Evidence:**
  - `compositor/src/appearance.rs` and `toolkit/platform/src/appearance.rs` both hold the same portal constants (`PORTAL`, `PORTAL_PATH`, `SETTINGS`, `NAMESPACE`, `PREFER_DARK`) and the same `ReadOne`/`ReadAll` plus `SettingChanged` loop.
  - The compositor opens six separate session connections (`main.rs:137,355,446`, `access_keys.rs:184`, `assistive.rs:48,123`) and two system connections (`logind.rs:42,64`). Each connection costs a socket plus zbus executor threads.
- **Change:**
  - Move a `portal_settings` module (`read_all(namespaces)`, `follow(namespaces, FnMut(ns, key, OwnedValue) -> bool)`) into `crates/libs/bus` (`bande-bus`, already the zbus helper crate). Both appearance modules become thin key mappers.
  - In `main.rs`, open one session and one system `Connection` and pass clones everywhere.
- **Lines:** about −50.
- **Risk:** low.

### 12. Split `toolkit/platform/src/window.rs` (2,564 lines)

| New home | Contents |
| --- | --- |
| `window/error.rs` | `WindowError` and its `From` impls (`:213-270`) |
| `window/layer.rs` | `LayerKind`, `Edges`, `LayerOptions`, `layer_of`, `anchor_of`, `keyboard_interactivity`, `set_layer_*`, `set_exclusive_zone`, `set_input_region`, `set_keyboard_focus` (`:87-127`, `:421-452`, `:1720-1836`) |
| `window/shm.rs` | `ShmTarget`, `ShmBuffer`, `create_buffer`, `copy_swizzled` (`:358-366`, `:1943-2176`) |
| `window/gpu.rs` | `Gpu`, `GpuSurface`, `RendererChoice`, `start_gpu`, `open_gpu` (`:367-382`, `:2201-2328`) |
| `window/menus.rs` | window menu, `show_menu`, `drive`, `draw_menus`, `serve_menus`, `apply_menu_outcome` (`:782-877`, `:1024-1370`) |
| `window/dispatch.rs` | all `Dispatch` impls (`:2329-2564`) |

Also:
- Break up `through_chrome` (`:1381-1584`, 200 lines in one function) by event kind.
- Replace `#[allow(clippy::too_many_arguments)]` on `ShmTarget::present` (`:1955`) with a borrowed `Target` struct.

- **Lines:** about 0 (moves).
- **Risk:** low.

### 13. One toolkit run loop instead of two
- **Evidence:** `run` (`toolkit/platform/src/run.rs:71-200`) and `run_surfaces` (`:358-420`) repeat the same per-window frame step: `wants_frame`, `plan_frame`, `scene`, `draw`, `wake_ui_at`, `animating`, then `request_redraw`, `set_text_input_area` and `wants_tree`, then `publish_accessibility` (`:105-120` against `:366-385`). They also repeat the same readiness and deadline assembly.
- **Change:** implement `Surfaces` for a one-window adapter around `App` and make `run` call `run_surfaces`, or extract `fn frame_step(window, app_hooks)`.
- **Lines:** about −80.
- **Risk:** medium; the run loop is core. `toolkit/platform/tests/{window,surfaces,input}.rs` cover it.

### 14. Errors that are swallowed without a reason
- **Toolkit, should log:**
  - `toolkit/platform/src/window.rs:1304,1616`: `let _ = self.draw_menus(frame)`, so a menu silently fails to appear;
  - `window.rs:1313`: `let _ = self.window_menu_action(..)`;
  - `window.rs:1364`: `open_submenu`;
  - `clipboard.rs:472,513,565`: `let _ = self.flush()`;
  - `clipboard.rs:879-883`: the transfer thread drops the spawn error and the write error.
- **Compositor, should log:** `popup.rs:115` and `ime.rs:64,69` ignore `track_popup` and `dismiss_popup` errors; `popup.rs:145` ignores `send_configure`.
- **Compositor, should check device capabilities first:** `pointing.rs:30-49` ignores five libinput config results. Check capability first (`config_tap_finger_count() > 0`, `config_dwt_is_available()`, and so on) so that real failures can be logged.
- **CLAUDE.md "no panics":** `state.rs:1390` has `unreachable!("every client is inserted with ClientState")`, which clippy's `panic` lint does not catch. Return a `static` empty `CompositorClientState` through `OnceLock`, or restructure so `insert_client` is the only constructor.
- **Lines:** about +25.
- **Risk:** low.

### 15. Localisation coverage tests
- **Evidence:**
  - The compositor's `every_message_id_used_in_code_is_in_the_english_file` (`compositor/src/lib.rs:94-103`) scans only `switcher.rs`. `access_keys.rs:189-200` uses eight IDs that are never checked.
  - `bande-widgets` (`L10N` in `secure_field.rs`, `editor.rs`, `combo_box.rs`, `search_field.rs`, `lib.rs`) and `bande-toolkit-platform` (`window.rs:784-835`, `chrome.rs`) have no such test at all.
- **Change:** add `access_keys.rs` to the compositor list, and add the same `bande_l10n::missing_ids` test to the widgets and toolkit-platform `lib.rs`.
- **Lines:** about +25.
- **Risk:** none.

### 16. Naming collisions and consistency with the Rust API guidelines
- **`HEADER_HEIGHT` means two things through one facade:**
  - window header, 48: `graphics/src/title_bar.rs:5`, re-exported as `bande_toolkit::HEADER_HEIGHT`;
  - table header, 28: `widgets/src/table.rs:18`, re-exported as `bande_toolkit::ui::HEADER_HEIGHT`.
  - Rename them `WINDOW_HEADER_HEIGHT` and `Table::HEADER_HEIGHT` (associated constant); update `apps/monitor/src/{monitor_view,inspector}.rs`.
- **`RenderError`:** exists in both `bande-render` and `bande-graphics`, and the compositor re-exports the first. Rename the graphics one `DrawError`, or the render one `VulkanError`.
- **Type naming:**
  - `Headless` (see task 8);
  - `ToggleFullscreen` against `ToggleFullScreen`: `compositor/src/input.rs` `Shortcut::ToggleFullscreen` against `bande_keyboard::Action::ToggleFullScreen`.
- **Parallel `_on` methods on `Headless`:** `take_redraw`/`take_redraw_on`, `latch`/`latch_on`, `present`/`present_on`, `render_elements`/`render_elements_on` (`state.rs:470-612`). Keep only the `_on(output)` forms, or index by screen.
- **Small cleanups:** a stray blank line in the `mod` list (`compositor/src/lib.rs:21`); `test-wayland/Cargo.toml` lacks `[lints] workspace = true` (add it with a crate-level `allow` for `expect_used` in test support).
- **Lines:** about −20.
- **Risk:** low.

### 17. Debian archive metadata
- **Licence:** `README.md:122` declares MIT OR Apache-2.0, but there is no `license` field in `[workspace.package]` or in any crate, and no `LICENSE-MIT` or `LICENSE-APACHE` at the root. Add `license = "MIT OR Apache-2.0"` plus `license.workspace = true` everywhere, and both licence files.
- **Rust version:** add `rust-version` to `[workspace.package]`. The code needs at least 1.88 (let-chains, `as_chunks`); the toolchain is pinned to 1.99.1.
- **Stale package description:** `compositor/Cargo.toml` `extended-description` still says "Headless Wayland compositor ... renders ... offscreen". Rewrite it for the DRM session compositor.
- **Unsorted dependencies:** the root `[workspace.dependencies]` are not sorted, against ADR 0003.
  - External: `alacritty_terminal` before `accesskit`, `image` before `flate2`, `md-5`/`kdl`, `serde`/`ruzstd`, the `naga`/`fontique`/`parley` block.
  - Internal: `bande-datetime` before `bande-controlcenter`.
- **Lines:** about +40.
- **Risk:** none.

### 18. Test-helper duplication
- **Evidence:**
  - 13 compositor test files define a one-line `fn app(..)` wrapper around `support::window` or `window_of` (`tests/headless/{gesture,assist,popup,dock,workspace,displays,tiling,motion,overview,window,..}.rs`).
  - `displays.rs:38` and `workspace.rs:13` redefine `chord`, which `support.rs:96` already has.
  - `check_golden` is written three times (`compositor/tests/headless/switcher.rs:202`, `widgets/tests/golden.rs:19`, `toolkit/platform/tests/support/mod.rs:64`).
  - `read_all` is written three times (`compositor/tests/headless/clipboard.rs:29`, `toolkit/platform/tests/appearance.rs:29`, `toolkit/platform/tests/support/mod.rs:27`).
- **Change:**
  - Call `support::window`/`window_of` directly.
  - Add `bande_golden::assert_golden_rgba!(name, (w, h), rgba)`.
  - Move `read_all` into `bande-test-wayland`.
- **Lines:** about −80.
- **Risk:** none.

### 19. Compositor `main.rs` follower boilerplate
- **Evidence:** the pattern "`match follow_x(bus, move |v| tx.send(v).is_ok()) { Ok => handle.insert_source(rx, |Event::Msg(v), _, app| ..), Err(e) => warn }`" repeats five times (`main.rs:381-427`, `451-465`).
- **Change:** one generic `fn follow<T>(handle, what: &str, start: impl FnOnce(Sender<T>) -> zbus::Result<()>, apply: impl FnMut(&mut App<O>, T))`.
- **Lines:** about −40.
- **Risk:** low.

### 20. Split the remaining oversized widget and compositor modules
Pure moves; tests stay where they are.

| File (lines) | Proposed split |
| --- | --- |
| `widgets/src/editor.rs` (1,781; one `impl` with 98 methods) | `editor/{history.rs (Edit, History, coalesce :76-139), selection.rs (Position, Selection :24-75), layout.rs (Block, layout cache, bounds), keys.rs (key and IME handling), mod.rs}` |
| `widgets/src/table.rs` (1,560) | `table/{model.rs (Column, Cell, Row, Line, flatten, Sort :33-200), paint.rs (paint_header, paint_selection, paint :1106-1312), mod.rs}` |
| `widgets/src/menu.rs` (1,321) | `menu/{model.rs (Menu, MenuItem, Search, collect_found), panel.rs (MenuPanel), tracker.rs (MenuTracker, inside_triangle)}` |
| `compositor/src/switcher.rs` (1,173) | `switcher/{layout.rs (lay_out, fitted, step and its tests :60-137, :1123-), paint.rs (paint_icons, placeholder, paint_name, card), mod.rs}` |
| `compositor/src/overview.rs` (1,193) | `overview/{layout.rs, paint.rs, mod.rs}`, the same shape |

- **Risk:** low. Merge-conflict risk is the only real cost.

### 21. Two spring-animation models
- **Evidence:**
  - Widgets use time-based `Animated` (`widgets/src/animation.rs`: `state_at(start, target, elapsed)`, interruptible).
  - The compositor uses step-based `bande_spring::Animation` advanced by frame deltas, in `gesture.rs:169`, `overview.rs:129`, `workspace.rs:153,172`, `reveal.rs:76`, `assistive.rs:157`, `zoom.rs:33`, `motion.rs:94-158` and `keymap.rs:78`.
  - Both sit on the same `Spring`. Step integration drifts with irregular vblanks.
- **Change:** move `Animated` (minus the `Context` glue) into `bande-spring` and migrate the compositor channels one module at a time.
- **Size:** larger than 2 hours as a whole; do one module per task, starting with `zoom.rs` and `reveal.rs`.
- **Risk:** medium (motion feel); the `tests/headless/motion.rs` timings guard it.

### 22. Remove `AsAny` with trait upcasting
- **Evidence:** `widgets/src/lib.rs:228-241` defines `AsAny` only so that `Ui::widget_as` (`:520`) can downcast. Rust 1.86 and later supports `&mut dyn Widget as &mut dyn Any` directly when `trait Widget: Any`.
- **Lines:** about −14.
- **Risk:** none.

### 23. Small hot-path cleanups in the compositor
- **Per frame:**
  - `render_screen` (`state.rs:614-638`) clears `drawn` for every window and then does `window_index(&surface)` per drawn surface, which is O(n²). Carry the window index in `Composed` instead.
  - `relocated()` (`state.rs:131-136`) reallocates the whole element vector every frame. Wrap the elements at creation in `compose_at`.
- **Per window:** `screen_of` (`screen.rs:242-251`) scans every workspace of every screen, and `presented()` (`workspace.rs:224`) calls it once more per call, several times per window per frame. Cache `screen` on `Window` when the workspace changes.
- **Toolkit:** `App::sources(&self) -> Vec<BorrowedFd>` (`toolkit/platform/src/run.rs:20`) allocates on every loop iteration. Return an iterator or a `SmallVec`-free slice.
- **Win:** small at today's window counts (under 20); it removes quadratic growth.
- **Risk:** low.

---

## Top 10

| # | Task | Why first | Estimate | Risk |
| --- | --- | --- | --- | --- |
| 1 | Hit-test once per pointer event; allocation-free `scene()` (task 1) | 2 to 4 scene rebuilds per mouse event today, each about 20 allocations and 4 mutex locks | −20 lines; about −40k allocations/s at 1000 Hz | low |
| 2 | One `Labels`/`TextSystem` for all compositor overlays; Abgr8888 upload (task 2) | 5 font collections and glyph caches in one process; duplicated paint and swizzle | −60 lines; estimated −4 to −12 MB RSS | low |
| 3 | Private protocols back to v1; delete the version gates (task 3) | Pure rot under the no-backward-compatibility rule | −150 lines, −51 XML attributes | low |
| 4 | `Window::draw(Scene)` by value (task 4) | Two whole-tree scene clones per toolkit frame | −5 lines; about −1,000 allocations per frame in large views | low |
| 5 | `gpu` feature in `bande-graphics` (task 5) | wgpu and vello_gpu are linked into a compositor that never uses them | estimated −2 to −5 MB binary; faster `-p` builds | low |
| 6 | Dead-code and dependency purge (task 6) | Dead toolkit capture and blur clients, unused API, a stray dependency | −360 lines | low |
| 7 | Switcher I/O off the compositor thread (task 7) | `.desktop` parsing and SVG rasterising on the frame thread | estimated −10 to −60 ms first-Alt+Tab stall | medium |
| 8 | Split `state.rs`; `Headless` becomes `Compositor`; test probes into `probe.rs` (task 8) | 1,794-line god module; misleading core name | `state.rs` under 500 lines | low (moves) |
| 9 | Shared `Rect::contains`, `resize_edge`; smithay `Rectangle` (task 9) | 4 copies of point-in-rect; 2 copies of resize zones | −90 lines | low |
| 10 | No CPU wait on the GPU fence per frame (task 10) | Compositor thread blocked for the GPU time of every frame | estimated 2 to 6 ms per frame back to input and clients | high |

Next in line: tasks 11 (portal follower and one bus connection), 12 (window.rs split), 13 (one run loop) and 14 (swallowed errors).
