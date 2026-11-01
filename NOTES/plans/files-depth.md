# Files depth

What a user needs from a file manager to rely on it every day, measured against the Finder.
Delete each line when it ships; delete the file when the list is empty.

## What users rely on

1. **Names they recognise**: the Finder hides known extensions by default (Settings ▸ Advanced
   "Show all filename extensions"), renaming edits the visible name and keeps the hidden
   extension, and changing an extension asks first. Shipped.
2. **Never lose work silently**: every copy, move, trash and delete either finishes or says
   which item failed and why (permission, read-only, disk full, disk gone) with Skip, Try Again
   and Stop; a copy that cannot fit is refused before it starts. Undo for every change.
3. **Get to things fast**: sidebar with the places they use (configurable as Settings ▸
   Sidebar), Go menu, Recents, a path bar, search that scopes sensibly, new windows on the
   folder they want.
4. **See what a file is**: Get Info (name and extension, kind, size, where, dates, permissions,
   Open With and Change All), Quick Look, previews and thumbnails.
5. **Arrange folders their way**: icons, list, columns (gallery), sort, groups, per-folder view
   options remembered, folders on top, hidden files.
6. **Other apps hand them to Files**: "Show in Folder" from browsers and editors
   (`org.freedesktop.FileManager1`), Open With and defaults, drag and drop with modifiers.
7. **Disks and servers**: external drives with Eject, Trash on volumes, Connect to Server.
8. **Stays fast**: large folders stream, nothing blocks typing, idle costs nothing.

## Already in Files

Icons, list (with disclosure) and columns; Quick Look with full screen; Get Info inspector with
Open With and default app; tabs; batch rename; Undo/Redo across the session's operations;
spring-loaded folders; Ctrl/Shift drag modifiers; Recents and search with scopes; per-folder
view memory; status bar and path bar; hidden files; Trash with Put Back, Empty, 30-day purge
and per-volume `.Trash-uid`; volumes with Eject; Compress and Extract; Open in Terminal; New
Document templates; Settings (General, Advanced).

## Open, ranked

1. Failure and empty-state slices F15–F23 (per-item failure alert with Skip/Try Again/Skip All,
   no-space check before a copy, permission-denied and empty-folder messages): on
   `origin/wip/files-depth-3`, written against the old `unbound_*` crate names; port onto
   `main` rather than merge.
2. `org.freedesktop.FileManager1` (ShowFolders, ShowItems, ShowItemProperties) so "Show in
   Folder" from Firefox, Chromium and editors opens Files with the item selected.
3. Settings ▸ Sidebar: which places show (Recents, Desktop, Documents, Downloads, Home, Trash,
   external disks); today only drag-out and "Restore Default Sidebar".
4. New windows show: Last Folder and Other… (any folder).
5. Get Info: an editable Name & Extension field with a per-file "Hide extension", and
   permissions editing (owner, group, others).
6. View ▸ Use Groups (Kind, Date Last Opened, Size) and View ▸ Sort By in the menu bar.
7. Undo across windows (one operation history for the process).
8. Gallery view.
9. In-place rename field text sits 5 px right of the label (the toolkit's in-place field keeps
   the 8 px text field padding).
10. Owner decisions: tags (colour labels in xattrs, a Tags settings tab), Connect to Server
    (GVfs-free SMB/SFTP mounts).
11. Content search (file contents, not only names).
