# Integrations: what makes macOS feel connected

Research date: 2026-10-29. Many Apple pages here were cited from their known URLs without
re-fetching in this pass. They are marked "(not re-fetched)" and should be spot-checked before
being quoted in a spec.

## What macOS does

### One preview, everywhere: Quick Look

- **Where it works.**
  - Space previews the selected file in Finder, open and save panels, Spotlight and Mail.
  - The preview zooms from the icon; arrow keys move between files.
  - It has a full-screen mode.
  - Markup, rotate and trim are available without opening an app.
  - <https://support.apple.com/guide/mac-help/view-and-edit-files-with-quick-look-mh14119/mac>
    (not re-fetched)
- **APIs.**
  - Any app can host the shared `QLPreviewPanel`.
  - Previews come from app extensions (`QLPreviewingController`); thumbnails come from
    `QLThumbnailProvider`.
  - <https://developer.apple.com/documentation/quicklook/qlpreviewingcontroller>
- **The lesson.** One system service and one key, in every place a file is shown.
- **Linux.**
  - GNOME Sushi offers `org.gnome.NautilusPreviewer.ShowFile(uri, window, close_if_shown)` on
    D-Bus. <https://gitlab.gnome.org/GNOME/sushi>
  - Thumbnails follow the freedesktop thumbnail spec.
    <https://specifications.freedesktop.org/thumbnail-spec/latest/>

### The document is an object: proxy icons and title bars

- **Proxy icon.**
  - Command-click the title for a path menu.
  - Drag the proxy icon to move, copy or attach the open document.
  - The title's menu offers Rename, Move To, Tags, Lock and Revert To.
  - Since macOS 11 the icon appears on hover; Accessibility > Display > "Show window title
    icons" makes it permanent (unverified details).
  - `NSWindow.representedURL` is all an app sets to get all of this.

### Moving content between apps

- **Clipboard.**
  - Each item carries several representations; the receiver picks one, and data can be
    promised lazily. <https://developer.apple.com/documentation/appkit/nspasteboard> (not
    re-fetched)
  - Paste and Match Style is Option-Shift-Command-V.
    <https://support.apple.com/en-us/102650>
  - macOS 26 adds clipboard history inside Spotlight (Command-Space, then 4), opt-in, with
    30 min, 8 h or 7 day retention.
    <https://macmost.com/how-to-use-the-spotlight-clipboard-history-in-macos-tahoe.html>
- **Services.**
  - Apps advertise actions on typed data in `NSServices`. They appear in the app menu and in
    the context menu of a matching selection.
    <https://developer.apple.com/documentation/bundleresources/information-property-list/nsservices>
  - Few users know it exists. The idea is great and the discoverability poor.
- **Share and Open With.**
  - Share (`NSSharingServicePicker`) has extensions.
  - Open With and default apps are run by LaunchServices: Get Info > Open with > Change All.
- **Screenshot thumbnail.**
  - After a capture, a thumbnail sits bottom-right for a few seconds (duration unpublished).
  - Click it for Markup, swipe it away, or drag it straight into a document.
  - <https://support.apple.com/guide/mac-help/take-a-screenshot-mh26782/mac>

### Text system everywhere

- Every Cocoa text field gets the same services for free, through `NSTextView` and
  `NSSpellChecker`. <https://developer.apple.com/documentation/appkit/nsspellchecker>
  - Spelling and grammar as you type.
  - Substitutions: smart quotes, dashes and user text replacements.
  - Look Up (force click, three-finger tap, Control-Command-D).
  - Emoji and symbols (Control-Command-Space or Globe-E).
  - Dictation.
  - Writing Tools.
- **The lesson.** The text system is one shared component, so every text field in every app
  behaves the same.

### Keyboard and focus across the system

- Ctrl-F2 focuses the menu bar, Ctrl-F3 the Dock, Ctrl-F4 the next window, Ctrl-F5 the toolbar
  and Ctrl-F8 the status menus. <https://support.apple.com/en-us/102650>
- "Keyboard navigation" makes Tab reach every control.
- Lists type-select: typing a prefix jumps to the item.
  <https://developer.apple.com/documentation/appkit/nstableviewdelegate/tableview(_:typeselectstringfor:row:)>

### Accessibility display options

Reduce Motion, Reduce Transparency, Increase Contrast and Differentiate Without Colour are
system switches. Apps read them from `NSWorkspace` and are notified on change.
<https://developer.apple.com/documentation/appkit/nsworkspace/accessibilitydisplayshouldreducemotion>
See animations.md for what Reduce Motion changes.

### Hubs

- **Spotlight** is a search box, and also:
  - A calculator.
  - Unit conversion.
  - Definitions.
  - Quick Look of results.
  - In macOS 26, app actions and clipboard history.
  - <https://appleinsider.com/inside/macos-tahoe/tips/spotlight-got-a-huge-upgrade-in-macos-tahoe-heres-how-to-use-it>
- **Notifications.**
  - Notifications have actions and inline reply.
    <https://developer.apple.com/documentation/usernotifications/untextinputnotificationaction>
  - Focus modes filter them by person and app.

### Continuity

Continuity covers Universal Clipboard, Handoff, AirDrop and Continuity Camera.
<https://support.apple.com/en-us/108046> (not re-fetched)

- On Linux, KDE Connect and GSConnect cover the useful part:
  - Clipboard sync.
  - Send file.
  - Notification mirroring with replies.
  - Remote input.
  - <https://kdeconnect.kde.org/>
- Handoff has no equivalent.

## What others do better

- **KRunner and GNOME search providers** let any app add results and actions to the launcher
  over D-Bus. This is more open than Spotlight's indexing before macOS 26.
  <https://develop.kde.org/docs/plasma/krunner/>,
  <https://developer.gnome.org/documentation/tutorials/search-provider.html>
- **Primary selection** (select, middle-click paste) is a Linux power feature macOS lacks.
  Unbound already serves it (`zwp_primary_selection_v1`).
- **Clipboard history on ext-data-control.** It existed on Linux long before macOS 26, though
  without privacy controls.

## For Unbound

1. **Quick Look as a system service** (now, new `unbound-preview` or in `crates/apps/files`).
   - Implement `org.gnome.NautilusPreviewer`-compatible `ShowFile`, so GTK apps can call it,
     and a toolkit call `preview(uri, source_rect)`.
   - Space in every file view and in the file chooser portal.
   - The window zooms from `source_rect` with `standard` and closes back into it. Arrow keys
     move without closing.
   - The preview window is a normal `xdg_toplevel` with a parent, so focus rules stay simple.
2. **Proxy icon in the title bar** (later, toolkit).
   - Apps set `represented_file`.
   - The toolkit title bar draws the icon on hover after 500 ms, as a drag source with
     `text/uri-list`. Super+click on the title shows the path menu.
   - It is one property for app authors and connects every document window to Files, Terminal
     and the Dock.
3. **Context menu actions on typed data, our Services** (later, toolkit, `unbound-launcher`).
   - Apps declare actions in their `.desktop` file (`Actions=`, plus `X-Unbound-Accepts=`
     MIME types).
   - The toolkit context menu for a selection shows matching actions under one "Actions"
     submenu, and the launcher indexes them.
   - It is the freedesktop way, discoverable because it lives in the context menu (DESIGN §2).
4. **Text services in the toolkit** (spelling now, the rest later, `crates/toolkit/widgets`).
   - Spelling through Enchant with hunspell dictionaries
     (<https://abiword.github.io/enchant/>), as you type, with suggestions in the context menu.
   - Then Look Up through a dictionary provider and an emoji picker on `Super+Ctrl+Space`.
   - Substitutions such as smart quotes are on in prose fields and off in code and terminal
     fields. The field declares its content type.
5. **Screenshot thumbnail** (now, `crates/shell/screenshot`).
   - The floating thumbnail, bottom-right, stays 5 s. It slides in with `standard` and leaves
     with `momentum` when flicked.
   - It is a drag source with `image/png` and `text/uri-list` (the file is written immediately,
     so no promise is needed).
   - Click opens it in the preview with markup later.
6. **Clipboard** (now in part).
   - Persistence exists (`unbound-clipboard`, behaviour.md).
   - Add Paste and Match Style (`Super+Alt+Shift+V`), which pastes `text/plain` only, in the
     toolkit.
   - History (later) belongs in the launcher as a filter, opt-in, with retention choices, as
     macOS 26 does.
7. **Accessibility switches** (now).
   - `config.kdl` gets `appearance { reduce-motion; reduce-transparency; increase-contrast }`.
   - The portal serves `reduced-motion` and `contrast` (standard keys). Reduce Transparency has
     no portal key: serve `org.unbound.appearance.reduce-transparency` and read it in the
     toolkit.
   - The compositor reads `config.kdl` directly.
8. **Keyboard reach** (now, compositor and shell).
   - `Super+F2` focuses the menu bar and `Super+F3` the Dock. Ctrl is the terminal's on
     Unbound, so these use Super.
   - Type-select in every toolkit list, with a 1 s reset (unverified macOS value).
9. **Launcher as hub** (later). Calculator and unit conversion in `unbound-launcher`, plus
   `org.gnome.Shell.SearchProvider2`-compatible providers so existing apps contribute results.
10. **Phone integration** (never, in the desktop core). Recommend KDE Connect-compatible apps; do
    not build Continuity.
