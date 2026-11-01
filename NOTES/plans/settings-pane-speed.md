# Plan: Settings panes slow to switch (Language & Time worst)

## Cause (reproduced)
`SettingsView` defers every rebuild while its own `pressed` flag is set (`view.rs:215`, set and
cleared only by PointerDown/PointerUp at :709-710, checked in `invalidate` :702 and `settle` :869).
A pop-up menu opens on PointerDown (`pop_up_button.rs:169`; Settings' context menus too). Its grab
sends the window a pointer leave and the release goes to the popup, so the view gets
PointerDown + PointerLeave and never PointerUp. The toolkit's `Ui` drops its capture on leave
(`widgets/src/lib.rs:794`), the view does not: `pressed` stays true, and from then on nothing is
rebuilt until a later click releases inside the content. So a pane chosen any way other than a
click (the clock menu's "Language & Time settings…" and "Notifications settings…", Control
Center's Wi-Fi, the View menu, arrow keys, `bande-settings <pane>`) raises the window still showing
the old pane, and live values stop updating, for as long as the owner does not click inside: seconds
to a minute. Since apps now stay running after their window closes, the stuck flag survives in the
parked view. Probe (scratch harness, live data): after PointerDown+PointerLeave,
`select_pane(LanguageTime)` and further service updates leave the old page on screen; the next
PointerUp shows it at once.

## Ruled out, with numbers (mini, release build, live system services read-only)
- No D-Bus on the UI thread: every service is read on its own thread (`follow.rs`). Live calls:
  timedated 79 ms on activation then 3 ms, `ListTimezones` 29 ms, localed 104 ms then 3 ms,
  hostnamed 81 ms, session services 3 ms. No timeouts or stalls in `journalctl --user`.
- All startup data arrives within 0.1 s warm, 0.3 s with the locale/zoneinfo/icon files evicted.
- UI-thread cost per pane switch (select + first scene): ≤ 7 ms for each of the 26 panes (Language &
  Time 2.6 + 4.2 ms, About 1.4 + 4.1). Headless compositor + the real app, private session bus,
  live system bus: first repainted frame 8-100 ms after an arrow key or a click, every pane reached.
- Remaining real costs, all small: layout picker after typing "e" 33 ms per keystroke (unbounded
  list, `pages.rs:886`), language picker "e" 16 ms (`language.rs:26`); `Layouts::load` parses
  evdev.xml lazily on the UI thread (4.6 ms, `view.rs:388`); `system_place` reads zone.tab on it
  (1 ms, `night.rs:79`); one full rebuild per update, not per batch (0.5-9 ms each).

## Fix, in milestones
1. **Stuck press (the cause).** Remove `SettingsView::pressed`; defer rebuilds only while the
   `Ui` holds a pointer capture (add `Ui::pointer_captured()` in toolkit widgets; it is already
   cleared on leave). Make `handle`/`perform` and the MenuAction/MenuClosed path report "changed"
   when `settle` rebuilt, so the runner always redraws. Check the same flag pattern in
   `widgets.rs:883` (thumbnail press) and other apps. ~30 lines.
2. **Coalesce updates.** `Settings::source_ready` applies every drained update, then rebuilds once;
   skip the rebuild when an update touches neither the shown pane nor sidebar availability or
   summaries. Guards against storms like the NetworkManager one fixed on 2026-11-01. ~40 lines.
3. **Bounded pickers.** Cap language/region/layout results like zones (`PICK_RESULTS`, ranked:
   prefix matches first) so typing stays under a frame. ~20 lines.
4. **Lists off the UI thread, cached.** Load `Layouts` and the system place in the locale worker
   once and send them with `Update::Locale`; cache the supported-locale list there instead of
   re-reading 324 files after every failed change (`locale.rs:259,287`). ~40 lines.
macOS reference: System Settings switches panes on mouse-down and shows the pane at once; data
fills in place. Milestones 2-4 keep that true under load; no placeholders are needed today since no
pane waits on a service.

## Risks
Milestone 1 changes when rebuilds run during a press; sliders, the display arrangement drag and
text selection must still keep their widget while dragging (the `Ui` capture covers exactly that).

## Verification
Owner on the mini: open a pop-up (e.g. Appearance), choose an item, then pick "Language & Time
settings…" from the clock menu and press ↓ in the sidebar search: the pane shows at once. One
regression test in `tests/settings.rs` (it guards real logic): PointerDown, PointerLeave,
`select_pane`, assert the new pane is in the accessibility tree. ~25 lines.

## Size
M1 ~30 lines + test, M2-M4 ~100 lines, desktop repo only. No owner decision needed;
recommendation: ship M1 alone first, M2-M4 as one follow-up.
