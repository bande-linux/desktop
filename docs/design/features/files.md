# Files
Status: proposal, awaiting the owner's review.

## Purpose
Files (`bande-files`) is where people see, open, organise and move their documents, and it is
the browser inside every Open and Save dialog through the portal FileChooser. It should feel as
calm as the Finder and be faster to drive from the keyboard, with one obvious way to do each
thing and the rest behind the context menu, a modifier or the inspector.

Modifier names follow the decided mapping: Super acts as Command. Alt is assumed to stand in for
Option (not yet recorded in `docs/architecture/overview.md`). Ctrl is free in Files.

## Reference survey

| Feature | macOS | GNOME / KDE / Windows | Note |
|---|---|---|---|
| Sidebar | Favorites, Locations, Tags; Recents and Shared pinned above; drag a folder in to add; Favorites and Tags reorderable, Locations not [1] | Dolphin: Places panel [15] | Removing a sidebar item never touches the folder [1]. |
| Views | Icons, list, columns, gallery; Command-1 to 4; per-folder or default view options (Command-J) [2][3] | Dolphin: icons, compact, details [15]; Windows: Gallery for photos [17] | Column view has no "Use as Defaults" [2]. |
| List columns | Control-click header to choose columns, drag to resize [2] | All three similar | |
| Path bar | Optional bottom bar, Option-Command-P [3] | GNOME and Dolphin: breadcrumb that turns into an editable location | |
| Go to folder | Shift-Command-G [3] | Typing a path in the location bar | |
| Search | Command-F; save as Smart Folder; attribute criteria [3][9][10] | Dolphin: filter bar narrows the current folder [15] | A live filter of the current folder is simpler than a query builder. |
| Quick Look | Space toggles; full screen; markup [6] | GNOME Sushi (unverified) | |
| Get Info / inspector | Command-I per-item windows; Option-Command-I one inspector following the selection (community source, unverified) | Dolphin: Information panel [15]; Windows: details pane, Alt-Shift-P [16] | A single panel that follows the selection beats many windows. |
| Open With and default app | Get Info > Open with > Change All [7] | | |
| New folder, duplicate | Shift-Command-N, Command-D [3] | | |
| Copy and move by drag | Drag moves; Option-drag copies [4]; same volume moves, other volume copies (secondary [12]) | GNOME: same device moves, other device copies; Ctrl copies, Shift moves [13] | |
| Cut and paste | Copy, then Option-Command-V moves [3] | Ctrl+X/Ctrl+V | macOS has no Cut for files. |
| Name conflicts | Stop, Replace, Keep Both; Merge for folders; Apply to All (secondary [12]) | | |
| Trash | Command-Delete; Put Back; Shift-Command-Delete empties [3][5]; optional removal after 30 days [5] | XDG Trash spec with per-volume trash folders [18]; GNOME restores from Trash [14] | |
| Undo | Command-Z, Shift-Command-Z [3] | Dolphin: undo of file actions [15] | |
| Tabs and windows | Command-T, Command-N; Merge All Windows [3][11] | Windows 11 and Dolphin tabs, tab dragging [16] | |
| Hidden files | Shift-Command-. (unverified on the fetched pages) | GNOME: Ctrl+H | Already in M22. |
| Tags | Seven colours, Control-1 to 7, sidebar section [8] | Stored as extended attributes, lost on many file systems and transfers [20] | |
| Eject | Command-E [3] | | |
| Network | Connect to Server, Command-K [3] | GNOME: Other Locations | |

## Proposed Bande feature set

### Now (essential)

**Window layout** — Sidebar on the left, toolbar on top, content in the middle, path bar and
status (item count, free space) at the bottom, optional inspector on the right.
Inputs: Super+Alt+S toggles the sidebar [3]; Super+Alt+I toggles the inspector.
Motion: panels slide in and the content reflows smoothly.

**Sidebar** — Two sections. Favourites: Home, Desktop, Documents, Downloads, Pictures, plus any
folder dragged in, reorderable by drag. Locations: the system disk, mounted volumes with an eject
button, Trash. Removing a favourite never touches the folder [1].
Inputs: left click opens; right click: Open in New Tab, Remove from Sidebar, Eject, Show in
Inspector; drag a folder in to add, drag to reorder; drop files on an item to move or copy there.
Motion: an inserted row slides in; the drop target highlights.

**Views: columns and list** — Columns (exists) and a list with sortable headers, disclosure
triangles to expand folders in place, and right-click on the header to choose columns
(Size, Kind, Date Modified, Date Added) [2]. One view setting per window, remembered per folder.
Inputs: Super+2 list, Super+3 columns [3]; click header sorts, again reverses; Right/Left expand
and collapse in the list.
Motion: switching views cross-fades; expanding a folder slides rows down.

**Toolbar** — Back, Forward, folder name, view switch, search field. Nothing else; actions live
in the context menu and the menu bar.
Inputs: Super+[ and Super+] go back and forward; Super+Up encloses [3].

**Path bar** — Always visible, each segment clickable and a drop target. Super+Shift+G, or typing
`/` or `~` in the view, turns it into an editable path with completion (GNOME behaviour,
unverified as documented).
Inputs: click a segment, drop on a segment, right click: Copy Path, Open in New Tab.

**Search** — Super+F focuses the field. Typing filters the current folder instantly; a scope
switch below extends it to the folder's subtree by name. Escape clears and returns. In the
content view, typing a name's first letters selects the match (type-to-select; Finder behaviour,
unverified on the fetched pages).
Inputs: keyboard; click a result to select, double click to open.

**Open** — Double click or Super+Down opens with the default app; folders open in place [3].
Inputs: double click, Super+Down, Return (see open questions).

**Open With** — Context menu submenu: the default app first, other apps whose desktop entry lists
the MIME type, then "Other App…" with a searchable list. The inspector's Open With pop-up has
"Use for all <kind> files", the macOS Change All [7], written to `mimeapps.list`.

**Context menu** — On any item or selection: Open, Open With, Quick Look, Show Inspector, Rename,
Duplicate, Copy, Paste, Move to Trash, Show in Enclosing Folder (in search results), Copy Path.
On empty space: New Folder, Paste, Sort By, Show Hidden Files. Alt swaps Move to Trash for
Delete Immediately.
Inputs: right click, press and hold, Menu key, Shift+F10.

**New folder, rename, duplicate** — Super+Shift+N creates "untitled folder" and starts renaming
it. Rename inline with the name selected without its extension. Duplicate (Super+D) makes
"name copy" beside the original [3].
Inputs: F2, or a slow second click on a selected name, starts renaming; Return commits, Escape
cancels.
Motion: new item fades in at its sorted position; a renamed item glides to its new position.

**Copy and move** — Super+C copies, Super+V pastes a copy, Super+Alt+V moves [3]. Dragging moves
within a volume and copies across volumes; Alt while dragging forces a copy [4]; Super while
dragging forces a move (unverified as macOS behaviour). The cursor badge shows "+" for copy.
Operations run in the background, never block the window, and can be cancelled.
Inputs: keyboard, drag, drop on folders, sidebar, path bar, Dock icons and other apps.
Motion: dropped items fly to their place; a cancelled drag flies back.

**Progress** — A progress row appears in the window's status area after 0.5 s with items,
bytes, time left and a cancel button; the same progress shows on the Files Dock icon through
`LauncherEntry` (see `dock.md`).
Motion: the row slides in and out; the bar animates.

**Conflicts** — One sheet per operation: Replace, Skip, Keep Both (adds a number), and for
folders Merge; "Apply to all" when more than one conflict is pending [12]. Stop cancels the rest.
The sheet shows both files' size and date with thumbnails.

**Trash and Put Back** — Super+Backspace moves to Trash [3] following the XDG Trash spec,
including per-volume trash folders [18]. In the Trash: Put Back restores to the original path
(recreating missing folders), Delete Immediately, Empty Trash (Super+Shift+Backspace, with a
confirmation) [3][5].
Motion: trashed items shrink away and the rest close up.

**Undo** — Super+Z and Super+Shift+Z for rename, move, copy (the copy goes to Trash), move to
Trash (Put Back), new folder and duplicate [3]. Session-wide, many levels, the menu item names the
action ("Undo Rename"). Nothing that deletes permanently is undoable, so it always asks first.

**Quick Look** — Space toggles; arrow keys move the selection while it stays open; Escape closes
[6]. Formats to reach in Now: common images, text and code, PDF first page, audio and video with
play. Super+Alt+Y opens it full screen [3].
Motion: zooms out of the item's icon and back into it.

**Inspector** — One panel that follows the selection (not one window per file): preview, name,
kind, size (folders computed in the background), dates, location, Open With, permissions;
multiple selection shows totals. Super+I shows it [3].

**Drag and drop** — Files are draggable to any Wayland app as `text/uri-list`, and drops from
apps create files. Hovering a dragged item over a folder for about 1 s springs it open (Finder
spring-loading, unverified on the fetched pages).
Inputs: drag, Alt and Super modifiers as above, Escape cancels.

**Watching folders** — Listings follow changes on disk (inotify) without flicker or losing the
selection.
Motion: added rows fade in, removed rows fade out.

**Hidden files** — Super+Shift+. toggles (exists).

**Removable media** — Volumes appear in the sidebar when mounted by `bande-automount`;
eject with the sidebar button, Super+E [3], the context menu or a drop on Trash. If a volume is
busy, the message names the app holding it (unverified as feasible for every case).

**Sorting** — By name (Finder order, exists), kind, size, date modified, date added; folders
sorted with files (see open questions).

**File chooser** — The portal FileChooser [19] reuses the sidebar, columns and list views,
search, path entry, Quick Look and New Folder. It omits Trash, rename and the inspector. Save
dialogs show a name field with the extension unselected, and a compact mode that expands to the
full browser.

**Keyboard map**

| Action | Keys |
|---|---|
| New window, new folder | Super+N, Super+Shift+N |
| Open, enclosing folder | Super+Down (and double click), Super+Up |
| Back, forward | Super+[, Super+] |
| List, columns view | Super+2, Super+3 |
| Inspector, Quick Look | Super+I, Space |
| Duplicate, rename | Super+D, F2 |
| Copy, paste, move here | Super+C, Super+V, Super+Alt+V |
| Move to Trash, empty Trash | Super+Backspace, Super+Shift+Backspace |
| Undo, redo | Super+Z, Super+Shift+Z |
| Search, go to folder | Super+F, Super+Shift+G |
| Home, Downloads, Documents, Desktop | Super+Shift+H, Super+Alt+L, Super+Shift+O, Super+Shift+D [3] |
| Hidden files, eject | Super+Shift+., Super+E |

### Later (power features worth having)

**Icon view** — Grid with thumbnails and an icon size slider, Super+1 [2][3]; shares its grid with
the desktop icons of M2. Moves to Now if the owner wants it (see open questions).

**Tabs** — Super+T, drag a tab out to make a window, drop a file on a tab to move it there [16].

**Recents** — A sidebar item listing recently used files from `recently-used.xbel`.

**Batch rename** — On a multiple selection: replace text, add text, number (`docs/design/catalogue.md` §1).

**Compress and extract** — Compress to `.zip` beside the selection; opening an archive extracts
it beside itself.

**Connect to Server** — Super+K for SFTP and SMB, shown under Locations.

**Saved searches** — Save a search to the sidebar as a live folder [9][10].

**Inspector extras** — EXIF, audio metadata, SHA-256 on request (`docs/design/catalogue.md` §1).

**Remove from Trash after 30 days** — Off by default, one checkbox [5].

**Quick Look markup** — Rotate and annotate images and PDFs [6].

### Never (left out)

**Gallery view** — Icon view with large thumbnails plus Quick Look covers it; a fourth view is a
concept too many.

**Grouping (Use Groups)** — Sorting already orders items; groups add a second axis few use.

**Per-folder view options panel (Super+J)** — A few remembered choices (view, sort, icon size)
replace a panel of options.

**Split view** — Tabs and two windows side by side (with snapping) do the same job.

**Many Get Info windows** — One inspector following the selection is simpler and handles
multiple items.

**Cut (Super+X) for files** — Copy then Move Here is explicit and never loses a file if the paste
never happens.

**Aliases created in Files** — Symbolic links are shown and followed, but making them stays in
the terminal; one kind of reference, the sidebar favourite, is enough for most people.

**Customisable toolbar** — One fixed, minimal toolbar; power lives in menus and keys.

## Open questions for the owner

1. **Return: open or rename?** The Finder renames on Return; GNOME, KDE and Windows open.
   Recommendation: Return opens (it is what most people expect in a list), F2 renames.
2. **Folders first or mixed?** The Finder mixes them by default. Recommendation: folders first,
   which Linux users expect and which makes columns easier to scan.
3. **Icon view in Now?** Recommendation: yes once the M2 desktop grid exists, since Pictures
   without thumbnails feels unfinished; until then list and columns.
4. **Tags?** Extended attributes are lost on FAT drives, many network shares and most copy tools
   [20]. Recommendation: leave tags out until there is a clear need; favourites and search
   cover most uses.
5. **Default view for new windows?** Recommendation: columns, Bande's signature view and the
   one with the best keyboard story; list is one key away.

## Sources
1. Apple, Customize the Finder sidebar on Mac: https://support.apple.com/en-gw/guide/mac-help/mchl83c9e8b8/mac
2. Apple, Change how folders are displayed in the Finder on Mac: https://support.apple.com/en-jo/guide/mac-help/mchldaafb302
3. Apple, Mac keyboard shortcuts: https://support.apple.com/en-us/102650
4. Apple, Drag and drop items on Mac: https://support.apple.com/guide/mac-help/drag-and-drop-items-mh35852/mac
5. Apple, Delete files and folders on Mac: https://support.apple.com/guide/mac-help/delete-files-and-folders-on-mac-mchlp1093/26/mac
6. Apple, View and edit files with Quick Look on Mac: https://support.apple.com/en-gb/guide/mac-help/mh14119/26/mac/26
7. Apple, Choose an app to open a file on Mac: https://support.apple.com/en-gb/guide/mac-help/mh35597
8. Apple, Tag files and folders on Mac: https://support.apple.com/guide/mac-help/mchlp15236
9. Apple, Search for files on Mac: https://support.apple.com/en-gb/guide/mac-help/mh15155/26/mac
10. Apple, Create or change a Smart Folder on Mac: https://support.apple.com/en-gb/guide/mac-help/mchlp2804/mac
11. Apple, Work with app windows on Mac: https://support.apple.com/en-kz/guide/mac-help/work-with-app-windows-mchlp2469/13.0/mac
12. MacMost, How to merge two folders on a Mac: https://macmost.com/how-to-merge-two-folders-on-a-mac.html
13. GNOME Help, Copy or move files and folders: https://help.gnome.org/users/gnome-help/stable/files-copy.html.en
14. GNOME Help, Files topics: https://help.gnome.org/users/gnome-help/stable/files.html.en
15. KDE, The Dolphin Handbook: https://docs.kde.org/stable_kf6/en/dolphin/dolphin/index.html
16. Pureinfotech, File Explorer details pane and tab dragging: https://pureinfotech.com/file-explorer-new-details-pane-tab-drag-windows-11/
17. Thurrott, File Explorer (24H2): https://www.thurrott.com/books/windows-11-field-guide/archived-22h2/312540/file-explorer-24h2
18. freedesktop.org, Trash specification: https://specifications.freedesktop.org/trash-spec/latest/
19. XDG Desktop Portal, FileChooser: https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.FileChooser.html
20. Eclectic Light, Are Finder tags useful metadata?: https://eclecticlight.co/2023/11/04/are-finder-tags-useful-metadata/
