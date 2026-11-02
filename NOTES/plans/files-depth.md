# Files depth

What a user needs from a file manager to rely on it every day, measured against the Finder.
Delete each line when it ships; delete the file when the list is empty.

## What users rely on

1. **Names they recognise**: the Finder hides known extensions by default (Settings ▸ Advanced
   "Show all filename extensions"), renaming edits the visible name and keeps the hidden
   extension, and changing an extension asks first. Shipped.
1. **Never lose work silently**: every copy, move, trash and delete either finishes or says
   which item failed and why (permission, read-only, disk full, disk gone) with Skip, Try Again
   and Stop; a copy that cannot fit is refused before it starts. Undo for every change.
2. **Get to things fast**: sidebar with the places they use (configurable as Settings ▸
   Sidebar), Go menu, Recents, a path bar, search that scopes sensibly, new windows on the
   folder they want.
3. **See what a file is**: Get Info (name and extension, kind, size, where, dates, permissions,
   Open With and Change All), Quick Look, previews and thumbnails.
4. **Arrange folders their way**: icons, list, columns (gallery), sort, groups, per-folder view
   options remembered, folders on top, hidden files.
5. **Other apps hand them to Files**: "Show in Folder" from browsers and editors
   (`org.freedesktop.FileManager1`), Open With and defaults, drag and drop with modifiers.
6. **Disks and servers**: external drives with Eject, Trash on volumes, Connect to Server.
7. **Stays fast**: large folders stream, nothing blocks typing, idle costs nothing.

## Already in Files

Icons, list (with disclosure) and columns; Quick Look with full screen; Get Info inspector with
Open With and default app; tabs; batch rename; Undo/Redo shared by all windows; View ▸ Sort By;
spring-loaded folders; Ctrl/Shift drag modifiers; Recents and search with scopes; per-folder
view memory; status bar and path bar; hidden files; Trash with Put Back, Empty, 30-day purge
and per-volume `.Trash-uid`; volumes with Eject; Compress and Extract; Open in Terminal; New
Document templates; failure alerts per item (Skip, Try Again, Skip All, Stop); a no-space
check before copying; empty-folder and no-permission messages; Settings (General, Show in
Sidebar, Advanced) with hidden extensions and Last Folder for new windows;
`org.freedesktop.FileManager1` ("Show in Folder" from other apps, D-Bus activated with
`--gapplication-service`); `bande-files --select <paths>`.

## Open, ranked

1. Get Info: an editable Name & Extension field with a per-file "Hide extension".
2. View ▸ Use Groups (Kind, Date Last Opened, Size).
3. New windows show: Other… (any folder).
4. Gallery view.
5. Owner decisions: tags (colour labels in xattrs, a Tags settings tab), Connect to Server
   (GVfs-free SMB/SFTP mounts).
6. Content search (file contents, not only names).
