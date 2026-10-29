# Drag and drop

Research date: 2026-10-29. Apple's HIG and AppKit pages render with JavaScript; AppKit facts
below were read from the documentation's JSON (`developer.apple.com/tutorials/data/...`) where
marked verified. "Unverified" marks the rest.

## What macOS does

### Where drag and drop works

macOS feels connected because nearly every visible object is a drag source and a drop target.
The places are:

| Source to target | Result |
|---|---|
| File from Finder to Terminal | Inserts the escaped path (unverified, well known) |
| File to a Dock app icon | Opens the file in that app; the icon highlights only if the app accepts the type (unverified) |
| File to the Dock Trash | Moves it to Trash. A disk dragged there turns Trash into Eject (unverified) |
| Text or image from an app to Finder or the Desktop | Creates a `.textClipping` or an image file (HIG, unverified) |
| Document from the title bar proxy icon | Moves, copies or attaches the open document (unverified) |
| File onto a proxy icon or folder while hovering | **Spring-loading**: the folder or window opens under the pointer after a delay, and the drag continues inside it. Space opens it at once. <https://www.oreilly.com/library/view/mac-kung-fu/9781941222799/f_0234.html>. The delay is a Short–Long slider in Accessibility; no default in ms is published. <https://macpaw.com/how-to/use-spring-loading-on-mac> |
| Window to another Space via Mission Control | Moves the window; onto a full-screen thumbnail it creates Split View. <https://support.apple.com/guide/mac-help/view-open-windows-spaces-mission-control-mh35798/mac> |
| Item to a window behind | Hovering over a Dock icon or a window during a drag brings it forward (spring-loading, unverified for windows) |
| Screenshot floating thumbnail to a document or folder | Drops the image before it is even saved. <https://support.apple.com/guide/mac-help/take-a-screenshot-mh26782/mac> |
| Mail attachment and similar to Finder | **File promise**: the file is written only when dropped (`NSFilePromiseProvider`). <https://developer.apple.com/documentation/appkit/nsfilepromiseprovider> |

AppKit makes spring-loading a first-class destination protocol: "dragging an object onto a
destination object and hovering or force-clicking to activate the destination object" (verified,
<https://developer.apple.com/documentation/appkit/nsspringloadingdestination>).

### Visuals

- The drag image is translucent and follows the pointer. Multiple items "flock" into a
  formation: default, none, pile, list or stack. Verified as an enum,
  <https://developer.apple.com/documentation/appkit/nsdraggingformation>.
- A count badge shows the number of items (unverified).
- The cursor carries the operation:
  - Green `+` for copy.
  - Curved arrow for alias.
  - Forbidden sign for no drop.
  - The "poof" cloud for removing from the Dock (unverified).
- **Slide-back on refusal.** `NSDraggingSession.animatesToStartingPositionsOnCancelOrFail`:
  "whether the dragging image animates back to its starting point on a cancelled or failed
  drag" (verified;
  <https://developer.apple.com/documentation/appkit/nsdraggingsession/animatestostartingpositionsoncancelorfail>).
  Duration unpublished.
- Destinations highlight when they would accept the drop. Lists show an insertion line between
  rows; a folder row highlights when the drop would go into it.
- Elements in a reorderable list (Dock icons, Finder sidebar) move apart to open a gap under the
  pointer, so the drop target is visible before the drop.

### Operations and modifiers

- **Default rules:**
  - On the same volume, a drag moves; to another volume, it copies.
  - Option forces copy, and Command forces move across volumes.
  - Option+Command makes an alias.
  - <https://support.apple.com/en-mo/guide/mac-help/mchlp1046/26/mac/26>
- Modifiers are read live during the drag; the cursor badge updates when you press or release
  them.
- Escape cancels and the image slides back (unverified).
- `NSDragOperation` values: none, copy, link, generic, private, move, delete (raw values
  unverified). Delete is how the Trash says it will consume the item.

## Wayland

- **Core flow** (`wl_data_device` version 3+, <https://wayland.app/protocols/wayland#wl_data_offer>):
  - **Source:**
    - The source creates a `wl_data_source`, offers MIME types and calls `set_actions`
      (copy, move, ask).
    - Then it calls `start_drag(source, origin, icon, serial)` with an optional icon surface.
  - **Target:**
    - The target receives `enter`, `motion` and `leave`.
    - It calls `accept(mime)` and `set_actions(supported, preferred)`.
    - On `drop` it calls `receive`, then `finish`.
  - **Source completion:**
    - `dnd_drop_performed` means the user dropped.
    - `dnd_finished` means the target is done, and a move may delete the original.
    - `cancelled` means refused, outside or timed out.
  - **The compositor chooses the action.**
    - The spec says Shift selects move and Control selects copy.
    - Otherwise it is "implementation-dependent".
    - This is where we put Unbound's Alt-copy rule.
- **MIME types that matter:**
  - `text/uri-list` (files, RFC 2483).
  - `text/plain;charset=utf-8`.
  - `image/png`.
  - `application/vnd.portal.filetransfer` (the sandboxed file hand-off through the
    FileTransfer portal).
  - `x-special/gnome-copied-files` (GNOME's cut and copy marker).
  - XDS (`XdndDirectSave0`) is the X11-era file promise. It has no Wayland standard
    equivalent; a source can offer a promise as a MIME type and write on `receive`.
- **`xdg-toplevel-drag-v1`** (staging, <https://wayland.app/protocols/xdg-toplevel-drag-v1>):
  - It attaches a toplevel to a drag, so a tab torn out of a window becomes a window that
    follows the pointer and can drop back into a tab bar.
  - Supported by KWin 6.7, Mutter 51 and Jay. Not supported by niri, COSMIC, Sway or Hyprland.
  - Firefox and Chromium tab tear-off are believed to use it (unverified).
  - Smithay ships no module for it (unverified; none listed at
    <https://docs.rs/smithay/latest/smithay/wayland/index.html>).
- **`wp_cursor_shape_v1`** has the CSS names `copy`, `alias`, `no-drop`, `grab`, `grabbing`
  and `move`, so drag cursors need no new protocol.
  <https://wayland.app/protocols/cursor-shape-v1>

## What others do better

- **Windows** shows a drop description under the cursor ("Move to Documents", "Copy to USB").
  <https://learn.microsoft.com/en-us/windows/win32/api/shlobj_core/ns-shlobj_core-dropdescription>
  (unverified) It is more discoverable than a green plus and fits DESIGN's "every pointer state
  is visible".
- **KDE Dolphin** pops a Move / Copy / Link / Cancel menu on drop when no modifier is held
  <https://docs.kde.org/stable5/en/dolphin/dolphin/>. It is powerful but slows every drop. The
  Wayland `ask` action exists for this. We should not make it the default; at most offer it on
  a secondary-button drag.
- **niri** scrolls the view when a drag nears a screen edge (30 px zone, 100 ms delay, up to
  1500 px/s) and switches workspace at the top or bottom edge (50 px, 100 ms).
  <https://github.com/YaLTeR/niri/wiki/Configuration:-Gestures> (via the gestures research,
  partly unverified). This is the Wayland-native form of "drag between Spaces".

## Current state in Unbound

- `wl_data_device_manager` is served (`state.rs`). The client and server DnD grab handlers use
  Smithay's defaults (`impl ClientDndGrabHandler for State {}`).
- We found no drag-icon rendering in `render.rs`, no modifier-to-action mapping, no slide-back
  and no spring-loading.
- `wp_cursor_shape_v1` is implemented.
- The toolkit has no drag source or target, no hover per widget and no right button
  (`docs/architecture/toolkit.md`).

## For Unbound

1. **Compositor drag session** (now, `crates/platform/compositor`).
   - Render the client's drag icon surface at the pointer, above everything, including layer
     surfaces.
   - Choose the action from modifiers and the shared mask:
     - Alt means copy (DESIGN: "Alt-drag copies").
     - Shift means move.
     - Alt+Super is reserved for "make link", the counterpart of Option+Command's alias.
       Wayland has no link action, so it waits until Files needs it (later).
     - No modifier means the target's preferred action.
   - Re-evaluate on every modifier change and send `action` events immediately.
   - The Wayland spec suggests Control for copy. Unbound reserves Ctrl for terminals, so use
     Alt and document the deviation in `behaviour.md`.
   - **Slide-back:** on `cancelled`, or a drop with action none, animate the icon from the
     pointer back to the drag origin with `standard`, then fade it with `fade`. The compositor
     knows the origin from `start_drag` and the grab start position.
   - Escape cancels the drag.
2. **Shell drop targets** (now, `crates/shell/dock`).
   - The Dock accepts `text/uri-list`. An app icon highlights only if the app's desktop entry
     lists a matching MIME type.
   - Trash takes files, and becomes Eject for a mount.
   - Hovering an app icon for the spring-load delay activates that app and raises its windows,
     so the drag continues into it.
   - Dock icons part with `quick` to show the insertion point when a file or app is dragged
     into the kept area.
3. **Spring-loading** (now in Files, later elsewhere; toolkit `DropTarget::spring_load`).
   - Delay 700 ms by default. Apple's default is unpublished; 0.5 to 1 s balances accidental
     opens against waiting. One Accessibility setting changes it.
   - Space opens at once.
   - Before opening, the target "blinks" twice (a `quick` highlight pulse) so the action is
     announced.
   - Folders in Files lists, sidebar items, tab bars, Dock icons and window thumbnails in the
     overview all use the same delay.
4. **Drop description** (now, toolkit and compositor).
   - The drag cursor carries a short label of what will happen: "Copy", "Move to Trash",
     "Open with Terminal".
   - Wayland has no protocol for it. Shell targets (Dock, overview) and toolkit targets can
     draw the label themselves inside their surface near the pointer. Our own source and
     target are both the toolkit, so this works across our apps without protocol work.
   - Cursor shapes `copy`, `alias` and `no-drop` cover third-party apps.
5. **Text and files into Terminal** (now, `crates/apps/terminal`). Accept `text/uri-list` and
   paste shell-quoted paths separated by spaces; accept `text/plain`.
6. **Clippings and file promises** (later, Files).
   - Accept `text/plain` and `image/png` drops into a folder by writing
     `Text Clipping – <first words>.txt` or `Image <date>.png`.
   - For promises between our apps, define `application/x-unbound-file-promise` (the source
     writes into a directory the target names in a follow-up exchange). Mail and screenshot
     thumbnails need it.
7. **Multi-item drag image** (now, toolkit). Up to 3 stacked thumbnails, offset 3 px, a count
   badge from 2 items, and a drag image at 70 % opacity. Lists show an insertion line 2 px in
   the accent colour; folders highlight with an accent fill at 20 %.
8. **Drag between workspaces** (later, with workspaces).
   - Holding a window drag or a file drag at the left or right screen edge for 300 ms switches
     the workspace with `standard`.
   - In the overview, workspace thumbnails are drop targets with spring-loading.
9. **`xdg-toplevel-drag-v1`** (later). Implement it in the compositor ourselves, since Smithay
   lacks it, once browsers or our own Files tabs need tear-off. GNOME and KDE support it, so
   Firefox and Chromium will use it.
10. **Thresholds** (now, toolkit).
    - A drag starts after 4 px of movement with the button down. Apple does not publish its
      value; iOS uses 10 pt for touch, and pointers need less.
    - Moving back within that radius before the threshold is still a click.
