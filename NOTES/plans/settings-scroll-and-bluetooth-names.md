# Settings: scrolling and Bluetooth device names

## Summary

Settings pages and the sidebar ignore the wheel and the touchpad because Settings uses its own
`Scroll` widget that has no event handler; only Page Up/Down and focus moves shift it. Bluetooth
rows show BlueZ's `Alias`, which for a device that never sent a name is its address with dashes,
and the device type (`Icon`) is read but never drawn. Fix: put Settings on the toolkit's
`ScrollState`, hide nameless nearby devices, and draw a type symbol on every device row.

## What the owner sees

- Bluetooth and Wi-Fi panes (and any long pane, the sidebar at large text) cannot be scrolled with
  the mouse wheel or two-finger touchpad. Page Up/Down works, but nobody knows that.
- Nearby devices during discovery are a list of `4C-87-5D-…`-style strings in alphabetical order,
  with no symbol, so the mouse cannot be told apart from phones, TVs and beacons.

## Findings

### Scrolling

- Settings wraps every page (`crates/apps/settings/src/pages.rs:1279-1284`) and the sidebar
  (`crates/apps/settings/src/header.rs:125-128`) in its own `Scroll`
  (`crates/apps/settings/src/widgets.rs:816-940`). It implements `measure`, `place`, `paint`,
  `access` and `children`, but no `handle`. The toolkit delivers `Event::Scroll` along the hovered
  path, deepest first (`crates/toolkit/widgets/src/lib.rs:787-792`); no Settings widget accepts it,
  so the wheel and touchpad do nothing. **Root cause.**
- The offset lives in `Rc<Cell<f32>>` on the view (`view.rs:216`, `219`) because Settings rebuilds
  the whole sidebar and content tree on every state change (`view.rs:1612-1627`). That is why the
  toolkit `ScrollView`, which owns its state, was not used.
- The only movers are Page Up/Down by a fixed 240 pt (`view.rs:43`, `850-860`) and a focus-follow
  in `Scroll::place` (`widgets.rs:866-891`) that runs on every layout. Once wheel scrolling exists,
  that follow would snap the page back to the focused control on every rebuild (Bluetooth
  rebuilds on each BlueZ signal during discovery, `services.rs:378-394`, settled every 200 ms).
- The toolkit already has everything needed: `ScrollState` (`crates/toolkit/widgets/src/scroll.rs:32-405`)
  does wheel springs, touchpad tracking with rubber band and fling, scrollbar drag and paging,
  a fading overlay scrollbar, AccessKit `ScrollUp/Down`, and `reveal`. Files, the launcher, Table
  and TextView keep a `ScrollState` outside the widget; Terminal's settings use `ScrollView`.
- `Ui::reveal_focus` (`lib.rs:1151-1167`) runs only on Tab (`lib.rs:1148`); programmatic focus via
  `Action::Focus` (`lib.rs:1221-1226`) does not reveal.

### Bluetooth names and types

- `Device::from_interfaces` (`crates/libs/bluetooth/src/lib.rs:65-84`) takes `Alias`, falling back
  to the address. BlueZ always exposes `Alias`: the user alias, else `Name`, else the address with
  `:` replaced by `-` (bluez `src/device.c`, `dev_property_get_alias`). `Name` exists only when the
  device sent one. So "has a name" is: `Name` present, or `Alias` differs from the dashed address.
- `Icon` is read (`lib.rs:76`) but nothing draws it: `device_rows` builds a plain
  `Row::new(Text(name), control)` (`pages.rs:584-661`), and `Row` has no symbol slot
  (`widgets.rs:1007-1040`). BlueZ derives `Icon` from the Class of Device (classic) or GAP
  Appearance (LE): `input-mouse`, `input-keyboard`, `input-tablet`, `input-gaming`,
  `audio-headphones`, `audio-headset`, `audio-card`, `phone`, `computer`, `camera-photo`,
  `camera-video`, `printer`, `scanner`, `modem`, `network-wireless`, `video-display`,
  `multimedia-player`. All have `-symbolic` icons in the Unbound or Adwaita theme on this machine
  (checked under `/usr/share/icons`); `bluetooth-symbolic` is the fallback.
- Nearby devices are sorted by name (`lib.rs:146-160`), so dashed addresses mix with real names.
- Live state (`busctl`, read-only): BlueZ 5.82, adapter `hci0` powered, **no device objects** at
  the moment, nothing paired and no discovery cache. So the mouse is not paired; the owner meets
  the unnamed list while trying to pair it. LE mice in pairing mode advertise a name (often in the
  scan response, which BlueZ's active scan collects), so hiding nameless devices plus a type
  symbol makes the mouse findable.
- Others: GNOME hides unknown discovered devices from its list (gnome-bluetooth MR 36, issue 44);
  macOS lists only named nearby devices; Blueman has an open "hide devices with no name" request;
  Windows 11 hides uncommon devices unless "Bluetooth devices discovery: Advanced" is set.
- The Settings spec forbids expanders and "Advanced" sections (`docs/experience/settings.md`
  §2 item 6, §8 item 5), so grouping nameless devices under a disclosure is off-pattern.
- Fake BlueZ (`crates/testing/test-bus/src/bluez.rs:111-262`) has no `Name` property, no nameless
  device and no `AddressType`; its consumers are `libs/bluetooth/tests/bluez.rs`,
  `apps/settings/tests/panes.rs` and `apps/settings/tests/forget.rs`.

### Adjacent (not this item)

- WirePlumber logs "PipeWire's BlueZ SPA plugin is missing": `libspa-0.2-bluetooth` is not
  installed, and neither it nor `bluez` is in the image's package list (`images/build.sh:44-48`).
  Bluetooth headphones will pair but play no sound.

## Options

**Scrolling**

1. *Give Settings' `Scroll` a `handle` that nudges the `Cell<f32>`.* Smallest diff, but a second,
   worse scroll implementation: no touchpad phases, fling, rubber band, scrollbar drag or
   accessibility actions. Rejected (CLAUDE.md rule 3).
2. *Toolkit `ScrollView` over shared state.* Add a constructor that takes
   `Rc<RefCell<ScrollState>>`, so the state survives Settings' rebuilds; delete Settings' `Scroll`.
   One scroll behaviour across the desktop; small toolkit change.
3. *Stop rebuilding Settings' tree.* Correct long-term, far too large for this gap.

**Nameless devices**

1. *Hide silently* (GNOME, macOS). Clean list; a nameless device cannot be paired from Settings.
2. *Hide, with one muted caption under Nearby devices*: "3 devices without a name are not shown."
   Honest about the filter, no new control.
3. *Disclosure row to a subpage of unnamed devices.* Against the spec's no-expander, no-Advanced
   rule; a page of addresses helps nobody.

Paired devices are never hidden. A paired device without a name is labelled by its type noun
("Mouse", or "Unknown device") with its address as the detail line.

## Recommendation

Scrolling option 2. Bluetooth: compute `named` and a device kind in `libs/bluetooth`, hide nameless
nearby devices (option 1 or 2, owner's choice), and give `Row` a leading 16 pt symbol used by
device rows. Reveal the focused control only when focus moves deliberately, never on rebuilds.

## Implementation plan

Each milestone is one short branch from `main`, gated by fmt, clippy `-D warnings` and the
workspace tests, verified on the mini before merge.

### M1 — Toolkit: `ScrollView` over shared state (≈1 h)

- `crates/toolkit/widgets/src/scroll.rs`: `ScrollView` holds `Rc<RefCell<ScrollState>>`;
  `ScrollView::new` keeps its signature, add `ScrollView::shared(child, state)`. Measure the child
  with unbounded height as Settings does today (check that Terminal's settings golden stays put).
- `crates/toolkit/widgets/src/lib.rs`: make `Ui::reveal_focus` public.
- Tests, `crates/toolkit/widgets/tests/scroll.rs`: a wheel scroll on a shared `ScrollView`, then a
  fresh `ScrollView::shared` over the same state placed in the same frame, keeps the offset; a
  touchpad drag past the end rubber-bands and springs back on the rebuilt view.

### M2 — Settings scrolls with wheel, touchpad and keyboard (≈2 h)

- `crates/apps/settings/src/view.rs`: `scroll` and `sidebar_scroll` become
  `Rc<RefCell<ScrollState>>`; pane changes reset them (`view.rs:962`, `1238`). Page Up/Down call
  `scroll_to` by the viewport height; delete `PAGE_STEP`. Add `reveal_pending: bool`, set where
  Settings moves focus on purpose (`focus_next`, `focus_best_match`, sidebar `StepPane`); in
  `scene()` lay out, then `ui.reveal_focus()` if pending. Restored focus after a rebuild does not
  set it.
- `pages.rs:1279`, `header.rs:125`: use `ScrollView::shared`.
- `widgets.rs`: delete `Scroll`, `focused_frame` and the now unused scrollbar constant.
- Tests, new `crates/apps/settings/tests/scroll.rs` (900×720, Keyboard Shortcuts pane, which is
  long without fakes):
  - a wheel event over the page raises the ScrollView node's `scroll_y` and brings the last
    shortcut into the viewport;
  - the offset survives a rebuild (`apply(Update::Values(..))`);
  - Tab onto a control below the fold scrolls it into view;
  - after scrolling away from a focused control, a rebuild does not jump back;
  - at large text in a 900×480 window the sidebar scrolls with the wheel.
- Goldens: refresh only those that lose today's always-on scrollbar (overlay bar is hidden at
  rest); review each diff.

### M3 — `libs/bluetooth`: names and device kinds (≈1.5 h)

- `crates/libs/bluetooth/src/lib.rs`: `Device` gains `named: bool` and `kind: Kind`
  (`Mouse, Keyboard, Tablet, GameController, Headphones, Headset, Speaker, Phone, Computer, Camera,
  Printer, Display, Other`) parsed from `Icon`, with `Kind::symbol() -> &'static str`; drop the
  `icon: Option<String>` field. `nearby_devices` returns named devices only (and the hidden count
  if the owner picks the caption).
- `crates/testing/test-bus/src/bluez.rs`: optional `Name` property; add one nameless LE device
  (dashed-address alias, no `Name`, no `Icon`) and one renamed device (alias set, `Name` differs).
- Tests, `crates/libs/bluetooth/tests/bluez.rs`: the nameless device is not nearby; a user alias
  counts as a name; the fake mouse reads as `Kind::Mouse` and an unknown icon as `Other`; update
  the existing nearby list assertion.

### M4 — Settings device rows with symbols and fallbacks (≈1.5 h)

- `crates/apps/settings/src/widgets.rs`: `Row::symbol(Option<Image>)`, painted before the label,
  sized and tinted like the sidebar symbol (`ink2`).
- `pages.rs:584-661`: pass `self.symbol(device.kind.symbol(), …)`; a paired device without a name
  shows its kind noun with the address in the detail line. If chosen, the muted "N devices without
  a name are not shown" caption under Nearby devices.
- `crates/apps/settings/locales/en-US/unbound-settings.ftl`: kind nouns, "Unknown device", caption.
- `crates/apps/settings/src/state.rs`: hidden count in `BluetoothState` only if the caption is
  chosen.
- Tests: extend `panes.rs` `bluetooth_pairs_after_the_code_is_confirmed_in_settings` to assert the
  nameless fake device is absent (and the caption, if chosen); `golden.rs` `bluetooth()` reads its
  devices from `FakeBluez` as `forget.rs` does, so the golden finally shows paired and nearby rows
  with symbols; refresh `bluetooth-light.png`.

## Risks

- `ScrollView` measures its child with the available size; switching Settings to it can shift
  layouts. M1 keeps the unbounded measure; goldens catch the rest.
- During discovery every BlueZ signal (RSSI changes) triggers a full re-read and rebuild after
  200 ms. Shared scroll state keeps scrolling smooth, but the rebuild cost is unchanged; filtering
  `RSSI`-only `PropertiesChanged` is a follow-up if the mini shows load.
- A device that never sends a name cannot be paired from Settings (same as GNOME and macOS);
  `bluetoothctl` remains.
- Hiding the always-on scrollbar changes several goldens; each diff needs a look, not a blanket
  refresh.

## Decisions for the owner

1. Nameless nearby devices: hide silently, or hide with one muted line saying how many are hidden?
2. Add `bluez` and `libspa-0.2-bluetooth` to the image (`images/build.sh`) as part of this work,
   so paired headphones actually play audio, or track it separately?
