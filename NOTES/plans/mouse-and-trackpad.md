# Mouse and Trackpad settings

## Summary

Each device class already keeps its own scroll direction and speed in the config and the compositor. The gap is in Settings: one pane, two settings for the mouse, and a trackpad section that stays hidden on the mini because it has no touchpad. Plan: split the pane into **Mouse** and **Trackpad**, add the libinput options people expect (primary button, acceleration, dragging, scroll method, gestures), apply them only where a device supports them, and update the panes when a device is plugged in. 7 milestones.

## What the owner sees

- The mini has two USB receivers (Logitech, Microsoft Nano) and no touchpad (`/proc/bus/input/devices`, `/run/udev/data/c13:*`: `ID_INPUT_MOUSE=1` only). The user's `config.kdl` has no `mouse` or `trackpad` node, so the defaults apply.
- Settings ▸ Trackpad & Mouse therefore shows only the Mouse section: Natural scrolling and Tracking speed. The Trackpad section is hidden, so the owner sees no trackpad options at all, and nothing makes it clear that the scroll switch covers the mouse only.
- If a trackpad is paired from Settings ▸ Bluetooth, the Trackpad rows do not appear until the window loses and regains focus.

## Findings

**Config (`crates/services/config`)**
- `Trackpad` has `tap_to_click`, `natural_scrolling`, `speed`, `secondary_click`, `disable_while_typing` (`src/lib.rs:378-397`). `Mouse` has `natural_scrolling` and `speed` (`src/lib.rs:399-403`). They are parsed at `src/lib.rs:1192-1212` and `1233-1244`, with defaults like a Mac: trackpad natural on, mouse natural off.
- Each value is a `Setting` variant (`src/edit.rs:36-42`, written at `187-228`), a D-Bus property on `org.bande.Config` (`src/service.rs:829-947`), and a change signal (`src/service.rs:244-264`). The compositor applies a change only when the signal fires, so every new property needs its own line there.

**Compositor (`crates/platform/compositor`)**
- `pointing.rs:20-22` treats a device as a touchpad when `config_tap_finger_count() > 0`; anything else with the Pointer capability gets the mouse settings (`pointing.rs:24-51`).
- Today it sets tap, disable-while-typing, click method, natural scrolling and speed. It does not set left-handed, the accel profile, tap-drag, drag lock, the scroll method or middle emulation.
- Hot-plug works: `DeviceAdded` configures the new device and keeps it (`pointing.rs:54-67`, called from `main.rs:336`).
- Live apply works: `follow_input_settings` re-reads `config.kdl` on any `PropertiesChanged` (`input_settings.rs:48-70`). `apply_pointer_settings` then reconfigures every known device (`pointing.rs:69-78`), wired in at `main.rs:425-433` and `455`. The greeter uses the defaults (`main.rs:518`).
- Branch `day/platform-quality-2` (task 14 of `docs/quality/platform-toolkit.md:253`) adds `report(device, setting, result)` and logs every failed libinput call. This plan builds on that helper and keeps its rule: check `*_is_available()` (or the method list) before every call, so that a warning means a real failure.
- Gestures are fixed in `gesture.rs:16-20` and routed at `gesture.rs:398-454`:
  - three-finger horizontal swipe: workspaces;
  - three-finger vertical swipe: overview;
  - four-finger spread: show desktop;
  - four-finger pinch: launcher.

  None can be turned off. A gesture that is not claimed goes to the client (the `Route::Client` path).
- Wheel scrolling is passed through as is: `input.rs:456-471`, `SCROLL_PER_DETENT = 15` at `input.rs:33`. There is no scroll speed.
- libinput bindings: `input` 0.9.1 (Cargo.lock), with the `libinput_1_21` feature. It exposes accel profiles (Flat or Adaptive only), left-handed, tap drag and drag lock (both booleans), the scroll methods (TwoFinger, Edge, OnButtonDown), middle emulation, dwt/dwtp and the send-events mode. It does not expose three-finger drag (libinput 1.27) or the clickfinger button map. The system has libinput 1.28.1.
- Middle-button emulation is available only on devices without a middle button. Both live mice report BTN_MIDDLE (`KEY=1f0000…` and `ffff0000…`, bits 272-276), so a mouse row for it would do nothing on most mice.

**Settings (`crates/apps/settings`, `crates/libs/settings-panes`)**
- There is one pane, `PaneId::TrackpadMouse`: `registry.rs:36,65,86,117`, built at `registry.rs:1114-1153`, keyword `trackpad-mouse`, symbol `input-mouse-symbolic`.
- Device presence comes from scanning `/run/udev/data/c13:*` for `ID_INPUT_TOUCHPAD` and `ID_INPUT_MOUSE` (`src/pointing.rs:1-40`). The scan runs at start (`main.rs:234`) and on every focus-in (`main.rs:164`), never on hot-plug. It ignores `ID_INPUT_POINTINGSTICK`.
- Rows are hidden per section by device (`pages.rs:979-988`), and the pane by `available()` (`view.rs:605`). Search already skips unavailable panes (`view.rs:1261-1266`). Night Light's From/To rows show how to hide a row that depends on another (`pages.rs:476`).
- Client mapping: `client.rs:155-181` (proxy), `270-276` (read list), `433-445` (read), `572-591` (write).
- Tests: `tests/panes.rs:760-790` (rows appear by device) and `tests/golden.rs:411-430,660` (`trackpad-mouse-light.png`).
- `input-touchpad-symbolic` ships in Adwaita (`/usr/share/icons/Adwaita/symbolic/devices/`).
- The spec already plans this in part. `docs/experience/settings.md:791-804` (slice 6) adds `mouse { primary-button; acceleration }` and `trackpad { acceleration }` and marks scroll speed for later (`:562`). The sidebar spec lists one pane, "Trackpad & Mouse" (`:180`, `:230`).

**References.** macOS has separate Mouse and Trackpad panes. The Trackpad pane has tabs (Point & Click, Scroll & Zoom, More Gestures), and dragging style sits under Accessibility. GNOME's Mouse & Touchpad has primary button, per-device speed, acceleration and scroll direction, and for the touchpad secondary click, tap to click and two-finger or edge scrolling ([GNOME Help](https://help.gnome.org/users/gnome-help/stable/mouse.html.en)). The libinput options come from [libinput configuration](https://wayland.freedesktop.org/libinput/doc/latest/configuration.html).

## Options

**A. Static rows per class; the compositor applies only what each device supports.** Settings shows a fixed set of rows for each pane, chosen so that common hardware supports them all. The compositor checks each capability and skips the rest.
- Pro: no new interface, and it reuses the udev scan and the existing config flow.
- Con: a row can be dead on unusual hardware, for example Secondary click on a touchpad with physical buttons.

**B. The compositor publishes device capabilities over D-Bus.** A new `org.bande.Pointing` with a `Devices` property; Settings hides unsupported rows and gets hot-plug from the signal.
- Pro: exact, and it could also feed device names elsewhere.
- Con: a new bus name and interface in the compositor (on the `keyboard_monitor.rs` pattern), a second source of truth beside udev, and more test setup. It is not needed for the hardware this desktop targets (Apple and laptop clickpads, ordinary mice).

**Gestures.** (1) Leave them fixed. (2) An on/off switch per gesture. (3) Configurable finger counts. Option 2 matches macOS's More Gestures, and a gesture that is off simply reaches the app. Option 3 adds state to `route_gesture` for little gain.

## Recommendation

Option A with gesture option 2.
- **Mouse pane:** Primary button (Left, Right), Tracking speed, Pointer acceleration, Natural scrolling.
- **Trackpad pane:**
  - **Click:** Tap to click; Dragging (Off, Tap and drag, Tap and drag with lock), shown only while tap to click is on; Secondary click.
  - **Scroll:** Natural scrolling; Scroll with (Two fingers, Edge).
  - **Pointer:** Tracking speed, Pointer acceleration, Ignore the trackpad while typing.
  - **Gestures:** Swipe between workspaces, Overview, Show desktop, Launcher.
- **Leave out:** middle emulation (dead on mice with a wheel button), trackpad left-handed (rare, and GNOME and macOS do not offer it per touchpad), three-finger drag (not in `input` 0.9.1).
- **Hiding and hot-plug:** each pane is hidden when its class is absent. Settings watches `/run/udev/data` so the panes appear and vanish on hot-plug.

## Implementation plan

**Status (2026-11-01):** milestones 1-5 are on `main`. Milestones 6-7 (gestures) are next.


Each milestone is one branch from `main`, merged when the local gate is green. Start after `day/platform-quality-2` merges, so that `report()` is there.

1. **Config: pointer options (slice 6 plus dragging and scroll method).**
   - Change `services/config/src/lib.rs`:
     - `Mouse { primary_button: PrimaryButton, acceleration: bool }`;
     - `Trackpad { acceleration: bool, dragging: Dragging, scroll_method: ScrollMethod }`;
     - the keywords `primary-button "left"|"right"`, `acceleration #true`, `dragging "tap"|"tap-with-lock"|"off"`, `scroll-method "two-fingers"|"edge"`.
   - Defaults follow libinput: left, adaptive, tap, two fingers.
   - Add the `Setting` variants (`edit.rs`), and the properties and change signals (`service.rs`).
   - Tests:
     - `tests/config.rs`: parse the defaults, the values and an invalid keyword with its position;
     - `tests/edit.rs`: a default value leaves no trace;
     - `tests/service.rs`: each property round-trips, a bad keyword gives InvalidArgs, and `watch_changes` sees `PropertiesChanged` for every new property.
2. **Compositor: apply every option by capability.**
   - Split `pointing.rs` into `Class` (Touchpad, Mouse), a `Support` value read from the device and a pure `fn options(class, &Support, &PointerSettings) -> Vec<DeviceOption>`.
   - A thin `apply` maps each option to a libinput call through `report()`.
   - It covers:
     - left-handed (mouse);
     - the accel profile, when `config_accel_profiles()` contains it;
     - tap drag and drag lock, when `config_tap_finger_count() > 0`;
     - the scroll method, when `config_scroll_methods()` contains it;
     - the existing five options.
   - Unit tests in `pointing.rs` on `options`:
     - a mouse gets the mouse speed, natural scrolling, left-handed and the flat profile;
     - a touchpad without Edge keeps its scroll method;
     - drag lock is set only when dragging is `tap-with-lock`;
     - nothing is planned that `Support` lacks.
   - Check on the mini: toggle each option from `busctl --user set-property`, then `sudo libinput list-devices` shows "Left-handed: enabled" and "Accel profile: flat" on the Logitech mouse.
3. **Settings: split the pane into Mouse and Trackpad.**
   - Replace `PaneId::TrackpadMouse` with `Mouse` (`input-mouse-symbolic`, keyword `mouse`) and `Trackpad` (`input-touchpad-symbolic`, keyword `trackpad`), in that order in Devices.
   - Each pane gets its own summary and keywords in `settings-panes/locales/en-US/bande-settings-panes.ftl`.
   - `view.rs` `available()` uses `pointing.mouse` or `pointing.trackpad`. Delete the per-section device arms in `pages.rs:979-988`, and update `header.rs:244`.
   - Update `docs/experience/settings.md`: sidebar, pane table, slice 6, now 17 panes.
   - Tests:
     - rewrite `tests/panes.rs:760` as "the Trackpad pane appears only with a trackpad, the Mouse pane only with a mouse, and search skips the absent one";
     - replace the `trackpad-mouse-light` golden with `mouse-light` and `trackpad-light`;
     - `settings-panes/tests/search.rs`: "scroll direction" finds both panes.
4. **Settings: the new rows.**
   - Primary button, Pointer acceleration ×2, Dragging, Scroll with: SettingIds in `registry.rs`, the proxy, read and write in `client.rs`, labels and descriptions in the `.ftl` file.
   - Hide Dragging while Tap to click is off, on the Night Light pattern in `pages.rs`.
   - Group the Trackpad rows into Click, Scroll and Pointer sections.
   - Tests:
     - `tests/panes.rs`: clicking each new row sends the right `Command::Set`;
     - Dragging disappears when Tap to click turns off;
     - the goldens are refreshed.
5. **Settings: hot-plug.**
   - In `main.rs`/`follow.rs`, an `bande_watch::FolderWatch` (`Changes::Any`) on `/run/udev/data` rescans when a `c13:` entry changes and sends a new `Update::Pointing(PointingDevices)` through the feed. Drop the focus-in rescan.
   - `PointingDevices::scan` counts `ID_INPUT_POINTINGSTICK` as a mouse.
   - Tests in `tests/follow.rs`:
     - `scan` on a temp dir with a touchpad entry, a mouse entry and a keyboard entry gives the right flags;
     - a new `c13:` file in a watched temp dir produces an `Update::Pointing` with the trackpad flag set.
   - Check on the mini: unplug and replug the Logitech receiver with Settings open on Mouse.
6. **Gestures: config and compositor.**
   - Config `trackpad { workspace-swipe; overview-swipe; desktop-spread; launcher-pinch }`, all `#true` by default, with their properties and change signals.
   - `gesture.rs` `route_gesture` claims a gesture only when its switch is on (`self.pointing.settings.trackpad`), otherwise `Route::Client`.
   - Tests:
     - config and service round trips as in milestone 1;
     - `tests/headless/gesture.rs`: with `workspace-swipe` off, a three-finger horizontal swipe reaches the app and the workspace does not change; with `desktop-spread` off, a four-finger spread reaches the app.
7. **Gestures: Settings section.**
   - A Gestures section in Trackpad with four switches, each described ("Swipe left or right with three fingers"), plus the client mapping.
   - Tests: `tests/panes.rs` checks that each switch sends its `Command::Set`; the `trackpad-light` golden is refreshed.

## Risks

- **No touchpad on the mini.** Milestones 2 and 6 cannot be checked on real trackpad hardware here. The unit and headless tests cover the logic; the trackpad path needs a paired Magic Trackpad or the M2 MacBook to verify live.
- **A missing change signal.** A new property without its line in `service.rs:244-264` writes the file but never reaches the compositor. The `watch_changes` tests in milestones 1 and 6 guard this.
- **Downgrade.** An older build reading the new keys fails on an unknown child (`lib.rs:1207`, `1239`). Only a concern when rolling back.
- **Disable while typing.** libinput pairs it with internal keyboards only, so it does nothing for a Bluetooth trackpad with an external keyboard. The row stays, as on GNOME.
- **Golden churn.** Milestones 3, 4 and 7 each refresh Settings goldens. Keep each refresh in the milestone that causes it.

## Decisions for the owner

1. **Gestures.** Ship the on/off switches (milestones 6-7) now, or leave the gestures fixed for now?
2. **Mouse scroll speed.** Add it now? It means one extra milestone that scales wheel deltas, including `v120`, in `input.rs:456-471`, the way KDE does. The spec has it as "later".
