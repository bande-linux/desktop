# Files: depth and polish against the Finder

Status: research and build plan of 2026-10-30, branch `night/files-monitor-spec`. Slices F1–F7
were built on 2026-10-31 in `night/files-depth-1`; see "Built" under §6. It reads with the accepted proposal [`../design/features/files.md`](../design/features/files.md),
the principles in [`../design/DESIGN.md`](../design/DESIGN.md) and the design language
[`../design/datum.md`](../design/datum.md).

The owner's verdict: "Files is nice looking but quite simple vs Finder on Mac." The task is depth
and polish inside the app we have, not a new app or a new surface. Every item below either makes a
basic task possible, makes a common task faster, or removes a rough edge a Finder user trips over
in the first week.

**Effort.** S is one slice: one agent, 1–2 hours, one coherent commit series. M is 2–4 slices.
L needs several days or an owner decision first.

**Now, later, never.** Now means the ordered slices in §6, built in that order. Later means worth
having, but not before the slices are done. Never means left out on purpose, with the reason.

## 1. What Files does today

Checked in `crates/apps/files/src`, `crates/libs/fs`, `crates/libs/preview` and the goldens in
`crates/apps/files/tests/golden`.

| Area | What works | Where |
| :--- | :--- | :--- |
| Window | Header with back, forward, title, view switch (icons, list, columns) and search; sidebar; path bar and status (items, free space) at the bottom, hidden with Super+/ and remembered | `toolbar.rs`, `paint.rs`, `views.rs` |
| Sidebar | Favourites: Recents, Home, user folders, GTK bookmarks; Locations: mounted volumes with an eject button, Trash. Drag folders in, reorder, remove, restore the defaults | `sidebar.rs`, `context.rs`, `drag.rs` |
| Views | Columns (default, arrow-key driven), list (sortable Name, Date Modified, Size; Kind unsortable), icons with thumbnails, a size slider (32–256 pt) and band selection; view and icon size remembered per folder | `browser.rs`, `controls.rs`, `grid.rs`, `views.rs` |
| Tabs | Super+T, Shift+Super+[ and ], Ctrl+Tab; a tab bar from two tabs; dropping on a tab springs it open | `tabs.rs` |
| Selection | Click, Shift, Super, Super+A, band in icons, type-to-select | `browser.rs`, `view.rs` |
| Open | Double click, Return, Super+Down, Super+O; Open With submenu; Open in Terminal; middle click opens a folder in a new tab | `context.rs`, `menus.rs` |
| Operations | Copy, Cut, Paste, Move Here (Alt+Super+V), Duplicate, New Folder, New Document from Templates, Rename (F2, slow click, Tab to the next), Compress to zip, extract by opening an archive, Trash with Put Back, Delete Immediately (Alt), Empty Trash with a sound, Eject; all in the background with progress and cancel; Undo and Redo | `libs/fs/src/ops.rs`, `operations.rs`, `undo.rs` |
| Conflicts | Replace, Skip, Keep Both, Stop, Apply to all | `controls.rs` |
| Drag and drop | Drag to any app; drop on folders, sidebar, tabs and path bar segments; spring-loading after 0.9 s; Alt copies and Super moves (the compositor picks the action) | `drag.rs`, `platform/compositor/src/drag.rs` |
| Quick Look | Space toggles, arrows move the selection under it: images, text (first lines), folder counts, other files through installed thumbnailers (PDF, video) | `paint.rs`, `libs/preview` |
| Inspector | Super+I shows one panel that follows the selection: preview, kind, size (folders counted in the background), modified, where, permissions as text, link target, Open With with Use for All | `paint.rs`, `view.rs` |
| Search | Super+F; fuzzy name search over the `unbound-search` index; scope "this folder" or Home; results in a list with a Where column; Recents (Shift+Super+F) | `search.rs` |
| Go | Back, Forward, Enclosing Folder, Desktop, Documents, Downloads, Home, Recents, Go to Folder (Shift+Super+G, relative paths and `~`) | `menu_bar.rs`, `view.rs` |
| Watching | inotify on shown folders, settled for 80 ms, then the whole folder is read again | `libs/fs/src/watch.rs`, `browser.rs` |

**What is missing or rough, in short:** the column view shows nothing for a selected file; the list
view cannot open folders in place; Quick Look and the inspector are thin; there is no batch rename,
Copy Path, Open With ▸ Other or Merge; search matches names only; nothing that Files paints itself
(sidebar, columns, icons, path bar, inspector, Quick Look) moves or reaches a screen reader; each
window is its own process, so Undo does not cross windows and nothing answers
`org.freedesktop.FileManager1`; and every folder is read on the UI thread.

## 2. What the references do

Only what matters for the gaps. Sources are at the end.

- **Finder.** Four views; the column view shows a preview column for a selected file [F1]. The list
  view opens folders in place with disclosure triangles, Right and Left, and Option-click for all
  subfolders [F2]. Control-click on the header chooses columns [F1]. Quick Look browses several
  selected items with the arrow keys, has an index sheet (Command-Return), full screen, zoom
  (Command-+/−), rotate and an "Open with App" button [F3]. The preview pane (Shift-Command-P)
  shows metadata and Quick Actions [F4]. Batch rename offers Replace Text, Add Text and Format with a
  counter [F5]. Search takes `kind:` and `date:` tokens and can be saved as a Smart Folder [F6].
  Shortcuts include Option-Command-S (sidebar), Option-Command-P (path bar), Shift-Command-C
  (Computer), Control-Command-T (add to sidebar), Option-Command-V (move here), Command-Y (Quick
  Look) [F2].
- **Windows Explorer.** Details pane and preview pane, tabs that are restored at sign-in, archive
  creation in 7z and TAR besides zip, "Copy as path" [W1][W2].
- **GNOME Files.** Typing starts a search; filters for When, What and full text [G1]. F2 renames
  with the extension unselected; batch rename by template or find-and-replace [G2]. Properties edits
  permissions and "Allow executing file as program" (GNOME help, files-properties, not fetched).
- **Dolphin.** Information panel, filter bar, split view, terminal panel, bulk rename, per-folder
  view properties, service menus, compare files [K1].
- **freedesktop.** `org.freedesktop.FileManager1` (`ShowFolders`, `ShowItems`,
  `ShowItemProperties`) is how Firefox, Chromium and other apps "show in folder" [X1].

## 3. Cost at 100 000 items

Nothing was built tonight, so these are estimates to size the first slice, not results:

- A Python stand-in on this VM (Linux 6.12, arm64, warm cache) read a folder of 100 000 empty files
  in 51 ms and did the two `stat` calls per entry that `Entry::read` does in 234 ms.
- `Browser::show`, `open_child` and `reload_column` call `list()` on the UI thread, which does the
  same two `stat` calls per entry and then sorts with `finder_order`. Every inotify batch (80 ms
  settle) reads the whole folder again.
- The list view builds every row up front (`table_contents`): a `format_date` with a time-zone
  conversion, a size string and a path key per entry.
- `Column::marked` is a `Vec<usize>` searched linearly, so Select All followed by painting or a
  Shift-click scales with the folder.

Expect a freeze of a few hundred milliseconds when a 100 000-item folder opens, on every change to
it, and on Select All in the list view. Slice F1 measures it; F2–F4 remove it.

## 4. Gap table

### Speed at scale

| Gap | Why a user notices | Now/later/never | Effort |
| :--- | :--- | :---: | :---: |
| Folders are read on the UI thread | Opening a big Downloads, a photo dump or `/usr/lib` freezes the window; the Finder shows the first items at once | now | S |
| A change reads the whole folder again | A download or a build writing into a big folder makes the window stutter every 80 ms | now | S |
| Selection and list rows scale with the folder | Super+A, Shift-click and switching to the list view hitch on big folders | now | S |
| No number for big folders | `unbound-session-bench` measures Files on 400 files only; regressions go unseen | now | S |

### Views

| Gap | Why a user notices | Now/later/never | Effort |
| :--- | :--- | :---: | :---: |
| The column view shows nothing for a selected file | Half the window is empty (Datum §1 calls it out); the Finder shows a preview and the facts there | now | S |
| The list view cannot open folders in place | Comparing two subfolders or dragging between them needs two windows; Finder, Dolphin and Explorer users expect triangles and Right/Left | now | S |
| No column choice, Kind not sortable, no Date Created | Sorting Downloads by kind or finding what arrived today is a daily task | now | S |
| Sort is per window, not per folder | Downloads by date and Documents by name cannot both stick | now | S |
| Dates are absolute ("14 Mar 2026, 13:00") | The Datum mockup and the Finder say "Today, 09:12"; the year is noise this year | now | S |
| Search results force the list but the view switch still shows the old view | The switch lies; picking another view does nothing obvious | now | S |
| Column and list rows have different heights (28 and 32 pt) | Switching views changes density; Datum sets list rows to 32 | now | S |
| Small thumbnails in list and column rows | The Finder shows icon previews at every size | later | S |
| Groups (by Kind or Date Added) | Useful in Downloads and Recents; a second axis most people never touch | later | M |
| Gallery view | Icons, the preview column and Quick Look cover it; a fourth view is one concept too many | never | — |
| A View Options panel (Super+J) | Remembered per-folder choices replace a panel of options | never | — |

### Navigation and windows

| Gap | Why a user notices | Now/later/never | Effort |
| :--- | :--- | :---: | :---: |
| No entry for the whole disk | You cannot reach `/` or see the system disk's free space from the sidebar; the Datum mockup shows "Internal" | now | S |
| The sidebar cannot be hidden | Alt+Super+S does nothing; small screens lose 200 pt | now | S |
| No "Add to Sidebar" on a folder | Only drag adds favourites; Control-Command-T and the menu item are missing | now | S |
| Go to Folder does not complete | Typing `/usr/sh` and Tab does nothing; GNOME and the Finder complete paths, and typing `/` in the view does not open the field | now | S |
| Each window is a separate process | Undo does not reach a move made in another window, every Super+N is a cold start, tabs cannot move between windows | now | M |
| No `org.freedesktop.FileManager1` | Firefox "Show in Folder" and Activity Monitor "Show in Files" cannot select the item in a running window | now | S |
| Drag a tab out to a window, Merge All Windows | Expected from browsers and the Finder | later | S |
| Connect to Server (SFTP, SMB) | NAS and server users need it; needs the gvfs-or-FUSE decision (roadmap: Later) | later | L |
| Split view | Two windows snapped side by side do the same | never | — |

### Preview and information

| Gap | Why a user notices | Now/later/never | Effort |
| :--- | :--- | :---: | :---: |
| Quick Look shows only the selected item, the first lines of text, no zoom, no full screen, no "Open with" | Paging through ten photos or reading a log in Quick Look is the Finder habit | now | S |
| The inspector lacks Date Created, image size and a way to make a file runnable | Downloaded AppImages and scripts need the executable bit; `chmod` in a terminal is not an answer | now | S |
| Open With has no "Other…" | A file with no registered app shows "There is no app to open…" and stops | now | S |
| Tags | Extended attributes are lost on FAT drives and many copies (proposal, open question 4) | later | M |
| Media playback in Quick Look | Needs a decoder stack; the thumbnailer poster plus "Open with" covers most uses | later | M |
| Markup, rotate and trim in Quick Look | Own editors are not now (owner, 2026-10-30) | never | — |

### Operations

| Gap | Why a user notices | Now/later/never | Effort |
| :--- | :--- | :---: | :---: |
| No batch rename | Renaming 40 photos is one sheet in the Finder, GNOME and Dolphin, forty F2s here | now | M |
| No Copy Path | Pasting a path into a terminal or a chat is a daily task (Explorer "Copy as path", Finder Option-Command-C) | now | S |
| No Merge when a folder meets a folder | Replace throws away the other folder's files; the Finder offers Merge | now | S |
| Errors arrive after the fact as raw text in a 5-second notice ("Permission denied (os error 13) (3 more)"), and nothing checks free space before a copy | A copy fails halfway through and the message is gone before it is read; the Finder refuses up front, says how much space is missing, and asks at the first failure | now | S |
| The menu says "Undo", not "Undo Move of 3 Items" | You cannot tell what Super+Z will do (proposal: the menu names the action) | now | S |
| Copy progress only inside the window | Hidden behind other windows, a long copy is invisible; the bar shows progress for apps through LauncherEntry | now | S |
| Eject fails without naming the app that holds the volume | "Busy" without a culprit is a dead end | later | M |
| Remove from Trash after 30 days | One checkbox in the proposal; not before the basics | later | S |
| Service menus and Quick Actions as a framework | Built-in actions (compress, terminal) cover the common ones | never | — |
| Creating aliases or links | One kind of reference, the sidebar favourite, is enough (proposal) | never | — |

### Search

| Gap | Why a user notices | Now/later/never | Effort |
| :--- | :--- | :---: | :---: |
| No kind or date filter | "The PDF I downloaded last week" is the common search; GNOME has What and When, the Finder `kind:` and `date:` | now | S |
| Search with no results shows an empty table | No message, no "search Home instead" | now | S |
| Content search | Needs a full-text indexer and its own decision | later | L |
| Saved searches in the sidebar | Smart Folders; after filters exist | later | S |

### Keyboard, motion and access

| Gap | Why a user notices | Now/later/never | Effort |
| :--- | :--- | :---: | :---: |
| Nothing Files paints moves | Selection blinks instead of gliding, the inspector and Quick Look pop, views switch with a cut; DESIGN §3 and Datum §11 ask for motion everywhere | now | M |
| Sidebar, columns, icon grid, path bar, inspector and Quick Look are missing from the accessibility tree | A screen reader sees the toolbar and nothing of the default (column) view | now | M |
| The sidebar cannot be reached from the keyboard | Tab never leaves the content; arrows cannot move between places | now | S |
| Missing shortcuts: Super+Y, Alt+Super+S, Ctrl+Super+T, Shift+Super+C, Alt+Super+C | Muscle memory from the Finder fails silently | now | S |

### Empty states and consistency

| Gap | Why a user notices | Now/later/never | Effort |
| :--- | :--- | :---: | :---: |
| The list view, the Trash, Recents and an unreadable folder show blank space | A folder you may not read looks empty, which is a lie | now | S |
| Only light goldens | Dark mode regressions slip through; DESIGN §4: light and dark are equals | now | S |

## 5. Decisions

Where this plan departs from or settles [`../design/features/files.md`](../design/features/files.md):

1. **Batch rename moves from Later to Now.** It is in the Finder, GNOME and Dolphin, and it is the
   most visible "Files is simple" gap after the preview column.
2. **One process, many windows.** The toolkit already runs several windows per process
   (`run_surfaces`, used by Activity Monitor). It makes Undo span windows as the proposal asks, makes
   Super+N instant and lets Files own `org.freedesktop.FileManager1`. Drag a tab out stays Later.
3. **The preview column is the column view's own preview, not a fourth view.** Gallery stays never.
4. **Copy Path is the Alt alternative of Copy** (Alt+Super+C), as Delete Immediately is of Move to
   Trash: no new top-level item.
5. **"Allow running as a program" in the inspector,** with simple Access pop-ups for owner, group
   and others. No full ACL editor.
6. **Search filters are two pop-ups in the scope bar (Kind, Date),** not typed tokens and not a
   query builder: visible, one click, keyboard-reachable.
7. **Dates read relative in lists:** "Today, 09:12", "Yesterday, 21:03", "28 Oct, 16:20", and the
   year only when it is not this year. Full dates stay in the inspector.
8. **Rows are 32 pt in the list and the columns** (Datum §4), the sidebar stays 28.
9. **Open With ▸ Other… is a sheet with a searchable app list,** reusing the inspector's Use for All
   path to write `mimeapps.list`.

## 6. Build slices

Each slice is one agent for 1–2 hours, ends green on the gate in the night brief, and leaves Files
better on its own. Tests are behaviour tests that fail without the change (CLAUDE.md: no
tautologies). "Goldens" are headless renders in `crates/apps/files/tests/golden/`, light and dark.
"Review" means steps and `shot` lines appended to one growing scenario,
`ci/review/scenarios/files-depth.scenario`, recorded with `ci/review/run.sh` and copied to
`/home/mikael/.cache/unbound-agents/report/<branch>/`. Crates: `apps/files` unless named.

### Built (2026-10-31, `night/files-depth-1`)

- **F1.** `unbound-files-bench` (`crates/testing/bench`) drives `FilesView` in-process on a folder of
  100 000 files: open, list view, scrolling, Select All and one external change. It measures the
  UI-thread work that F2–F4 change; the frame time of the session stays with
  `unbound-session-bench`. Numbers in [`../architecture/performance.md`](../architecture/performance.md).
- **F2.** The first 2 000 entries are read at once and shown; a `Lister` thread (socket-pair
  wake-up like `Loader`) reads the rest in doubling chunks that merge in sort order. Selection,
  reveal and Go to Folder follow the entries as they arrive; leaving the folder drops the
  listing.
- **F3.** `FolderWatch` reports changed names; `Browser::apply` re-stats only those and inserts or
  removes their rows in place; a change to the folder itself or a queue overflow still reads it
  again. The list view applies the same row edits to its `Table` (`insert_row`, `remove_row`).
- **F4.** Marks stay sorted and are searched by bisection; the list view selects by row index
  (`Table::select_rows`, `selected_rows`) instead of 100 000 path strings; sizes and dates are
  formatted once per value; sorting re-orders in memory instead of reading again; the column
  view paints only its visible rows.
- **F5.** The preview column, 32 pt rows in the columns view, dark goldens for every view.
- **F6.** List rows of folders are expandable; Right, Left, Alt+Right and the disclosure open them
  in place, children are listed when opened (on the UI thread: an opened folder is usually small),
  watched while shown and kept while the window lives; selection and drag work across levels.
- **F7.** Kind is sortable (by extension, folders first: the type groups match the Kind column
  closely and need no MIME lookup while sorting), sort is remembered per folder in the views
  file, dates read relative in lists, a header right-click chooses Date Modified, Date Created,
  Size and Kind, remembered in `files-columns` beside the views file.
- Not done: the review scenario `files-depth.scenario` (the harness installs packages into the
  live system; the goldens in `crates/apps/files/tests/golden/` are the shots instead).

### F1. Measure big folders

Add a "big folder" set to `unbound-session-bench`: open a generated folder of 100 000 files, scroll
its list view, Select All, create one file in it while it is shown. Record the numbers as a row in
`docs/architecture/performance.md`. No change to Files.

- Tests: none beyond the bench running in CI's perf job.
- Acceptance: the table has before numbers for open-to-first-frame, frame p99 while scrolling,
  Select All, and one external change. Crates: `testing/bench`.

### F2. Read folders in the background

`Browser` asks a listing worker (a sibling of `Loader`, woken through the same socket-pair pattern)
for every folder it shows; results arrive in chunks of about 2 000 entries, sorted as they merge; a
newer request for the same column discards the older one. The column shows its first chunk in the
next frame; nothing waits on `read_dir`. Reveal and selection are applied when the entry arrives.

- Tests: `go()` into a folder of 20 000 files returns before the listing finishes and the column
  fills over later `source_ready` calls; going back mid-listing drops the stale result; a revealed
  file is selected once its chunk arrives; Go to Folder on a file still selects it.
- Acceptance: F1's open-to-first-frame at 100 000 items is under 50 ms; before and after numbers in
  the report. Crates: `apps/files`, `libs/fs`.

### F3. Apply folder changes, do not re-read

`FolderWatch` reports what changed (created, removed, moved from and to, attributes) with names;
`Browser` patches the column: binary insert in sort order, remove, update one entry. A queue
overflow or a change to the folder itself still re-reads everything.

- Tests: creating, deleting and renaming one file in a folder of 20 000 updates only that row and
  keeps the selection and the scroll position; a queue overflow falls back to a full read.
- Acceptance: F1's "one external change" costs under one frame (8 ms at 120 Hz) at 100 000 items.
  Crates: `libs/fs`, `apps/files`.

### F4. Selection and list rows at 100 000

Marks become a set (or a bit set) with the cursor and anchor beside it; the list view builds rows
without formatting every date (format on demand, or a small cache keyed by the minute); kind
strings are shared. If `Table` must learn lazy cells, stop and report the need from
`toolkit/widgets` rather than editing it.

- Tests: Select All then Shift-click at 100 000 entries keeps the right marks; sorting by size
  after Select All keeps them.
- Acceptance: F1's Select All and list-switch numbers under 16 ms; before and after numbers.

### F5. A preview column for a selected file

In the column view, a selected file (not a folder) opens one more column: the preview (image,
thumbnailer picture, or the first lines of text) centred at up to 256 pt with radius 6, the name in
Body strong, "Kind · Size" in Callout `ink2`, then Created and Modified as label and value rows, and a
quiet "More Info" button that shows the inspector. It reuses `Loader` and the Quick Look preview.
Several selected files show "N items · total size". Column rows move from 28 to the Datum 32 pt
of the list view (§5.8).

- Tests: selecting a file adds a preview column with its kind and size; selecting a folder shows its
  children instead; Left from the preview column returns to the file; a click on the second row
  lands on the second entry at the 32 pt pitch.
- Goldens: `columns-preview` (an image selected) light and dark; this slice also adds dark variants
  of the four existing goldens.
- Review: select a PDF and a photo in the column view; shot each.

### F6. Open folders in place in the list view

Folders in the list view get a disclosure triangle (the `Table` already supports child rows,
indent and disclosure). Click or Right expands, Left collapses or moves to the parent row,
Alt+Right expands every subfolder, Alt+click on the triangle does the same. Expanded folders are
watched, keep their state while the window lives, and can be selected and dragged across levels.

- Tests: Right on a folder shows its children indented under it, Left hides them; an expanded
  folder's new file appears without collapsing; a multi-level selection drags all its paths.
- Goldens: `list-expanded` light and dark.
- Review: expand two folders, drag a file from one into the other; shot before and after.

### F7. Columns, Kind sort, relative dates, sort per folder

Right-click on the list header chooses columns: Date Modified, Date Created (statx birth time),
Size, Kind. Kind becomes sortable and appears in Sort By. The sort joins the view and icon size in
`ViewMemory`, so each folder keeps its own. Dates read as decided in §5.7. Column choice is
remembered beside the views file.

- Tests: Kind sort groups by kind then name; Downloads by date and Documents by name each come back
  after a restart; `format_date` gives Today, Yesterday, this year and older forms for fixed
  times; a hidden column stays hidden in a new window.
- Goldens: update `tabs-list` (relative dates) light and dark.

### F8. Quick Look depth

A header row in the panel: the name, "2 of 5" when several items are selected, an "Open with
<App>" quiet button, and a full-screen toggle (Alt+Super+Y). With several items selected, Left and
Right move within the selection only (the Finder's index sheet is left out). Text scrolls with the
wheel and Page Up/Down; while Quick Look shows an image, Super+= and Super+− zoom it instead of
resizing icons.

- Tests: with three files selected, Right moves Quick Look to the second without changing the
  selection; "Open with" asks for the default app; Page Down scrolls a long text preview; full
  screen covers the window and Escape returns.
- Goldens: `quick-look-text` and `quick-look-several` light and dark.
- Review: Quick Look on a selection of photos, step through, full screen; shots.

### F9. Inspector depth

Add Created, image dimensions (read from the header, no full decode), and for files you own:
"Allow running as a program" (a switch that sets the executable bits for whoever can read) and
Access pop-ups for You, Group and Everyone (Read & Write, Read only, No Access). Several items show
the count, total size and the kinds. Changes apply at once and refresh the panel.

- Tests: the switch makes a script executable and the listing reflects it; Access "Read only"
  removes the write bit for that class only; image dimensions are read from a PNG and a JPEG header.
- Goldens: `inspector-file` and `inspector-several` light and dark.

### F10. Open With ▸ Other…

The Open With submenu ends with "Other…", which opens a sheet: a search field and every app with
its icon, recommended apps first, then all others; an "Always Open With" checkbox writes the
default through the existing Use for All path. Opening a file with no app shows this sheet instead
of the notice.

- Tests: choosing an app from the sheet opens the file with it; with the checkbox, the default
  changes in `mimeapps.list`; a file with no app presents the sheet.
- Goldens: `open-with-other` light and dark.

### F11. Copy Path, Add to Sidebar, named Undo

Copy gets an Alt alternative "Copy “name” as Pathname" (Alt+Super+C), several items one path per
line. A folder's menu gets "Add to Sidebar" (Ctrl+Super+T). Show in Enclosing Folder also appears
in Recents. Undo and Redo name the action in the Edit menu ("Undo Move of 3 Items", "Redo
Rename"). Super+Y joins Space for Quick Look.

- Tests: Alt+Super+C puts the absolute paths, unquoted, on the clipboard as text; Add to Sidebar
  writes a bookmark; the Edit menu's Undo title follows the last action; Recents offers Show in
  Enclosing Folder.

### F12. Batch rename: the engine

A pure function in `libs/fs` turns names and a rule (Replace Text, Add Text before or after,
Format: a base name plus an index or counter starting at a number, before or after) into new
names, keeps extensions, and reports clashes with each other and with the folder. Applying it runs
as one undoable step.

- Tests: each rule on a fixed list; extensions kept; a clash is reported before anything changes;
  one Super+Z restores every name.

### F13. Batch rename: the sheet

With two or more items selected, F2 or Rename opens "Rename N Items": a pop-up for the rule, its
fields, and a live preview of the first three new names; Rename is disabled while a clash exists,
with the reason in `danger` text.

- Tests: F2 with several items presents the sheet; typing updates the preview; Rename applies and
  selects the renamed items.
- Goldens: `batch-rename` light and dark. Review: rename a set of photos; shots.

### F14. Search filters and a consistent search view

The scope bar gains two pop-ups: Kind (Any, Folder, Document, Image, Audio, Video, Archive, by MIME
major type and a short list of document types) and Date (Any Time, Today, Last 7 Days, Last 30
Days, This Year, on Date Modified). The view switch shows List and is disabled while searching.

- Tests: Kind Image keeps only images among name matches; Last 7 Days drops an older file; Escape
  restores the previous view and switch state.
- Goldens: update `search` light and dark.

### F15. Empty states and unreadable folders

Centred `ink3` Body text, no art: "Empty Folder" in every view, "The Trash is empty", "Files you
open appear here" in Recents, "No results for “text” in Folder" with a quiet "Search Home" button,
and "You don't have permission to see the contents of “Folder”" when `read_dir` is denied (today it
looks empty).

- Tests: a folder with mode 000 shows the permission message, not an empty list; a search with no
  results offers Home and widens the scope when clicked.
- Goldens: `empty-list`, `no-permission` light and dark.

### F16. Errors with a way out, and space before copying

Before a copy or a move across volumes, `libs/fs` adds up the sources and compares with the free
space of the destination; if it does not fit, the operation stops before writing with "“Name”
can't be copied because “Volume” needs 3.2 GB more space". Today failures are collected and
reported afterwards in a 5-second notice ("(3 more)"); instead, the first failure pauses the job
and asks in a sheet, like a conflict: Skip, Skip All, Stop, and Try Again where it can help. I/O
errors map to sentences for permission denied, read-only volume, no space, name too long and a
device gone.

- Tests: a copy larger than a small tmpfs is refused before any byte is written; a read-only
  destination gives the read-only sentence; Skip continues with the next item and Skip All skips
  later failures without asking. Crates: `libs/fs`, `apps/files`.

### F17. Merge folders, and a better conflict sheet

When a folder meets a folder of the same name, the sheet offers Merge (the default when both are
folders): items are copied in, inner conflicts ask again. The sheet shows both items with icon or
thumbnail, size and date, and "Newer" beside the newer one.

- Tests: Merge keeps the destination's own files and adds the source's; an inner conflict asks;
  Undo of a merge removes only what it added.
- Goldens: `conflict-folders` light and dark. Crates: `libs/fs`, `apps/files`.

### F18. Disk in the sidebar, and hiding the sidebar

Under Locations, before volumes: the system disk, named after the computer's pretty hostname or
"Internal", opening `/`, with Shift+Super+C. Alt+Super+S hides and shows the sidebar with a
`standard` slide (Reduce Motion: fade), remembered like the status bar.

- Tests: the disk row opens `/`; hiding the sidebar is remembered for the next window; the content
  frame widens by the sidebar width.
- Goldens: `sidebar-disk` light and dark.

### F19. Go to Folder that completes

Typing `/` in the content view opens Go to Folder with that character, and so does `~` unless a
name in the folder starts with it (type-to-select keeps working). Tab completes the
longest common prefix; up to eight matching folders list under the field, Up and Down choose,
Return goes. Hidden folders appear only when their name starts with a dot.

- Tests: `/us` Tab gives `/usr/`; `~/Doc` lists Documents; Return on a listed folder opens it.

### F20. Motion: the selection glides, panels slide

The selection block moves to the new row (`quick`) in the sidebar, the columns and the icon grid;
the inspector and the preview column slide in from their edge (`standard`); the view switch
cross-fades (`fade`). All retarget when interrupted; Reduce Motion turns movement into fades.
Uses the toolkit's animation module.

- Tests: with a fixed clock, the selection rectangle is between the two rows mid-animation and on
  the new row when settled; with Reduce Motion it never sits between rows.
- Review: record arrow-key presses and Super+I at 120 Hz; frames in the report show no dropped
  frames (frame times from the bench, p99 under 8 ms).

### F21. Motion: Quick Look and changing rows

Quick Look grows out of the item's icon and shrinks back into it (`standard`); a new item fades in
at its place; a trashed item fades and the rows below close up (`quick`); a renamed item glides to
its sorted place.

- Tests: Quick Look's frame starts at the item's icon rectangle; a removed row's space closes over
  the animation; Reduce Motion fades in place.
- Review: Space on an icon and Super+Backspace on a row; frames in the report.

### F22. Accessibility: sidebar, columns, path bar

Publish what Files paints: the sidebar as a list of items (name, selected, actions Click and Show
Menu), each column as a list with its items, the path bar as buttons. Tab moves focus sidebar →
content → toolbar; arrows move inside the sidebar; Return opens the place.

- Tests: the tree holds the sidebar items and the visible column items with names and selection;
  `perform` Click on a sidebar item opens it; Tab then Down then Return opens the second place.

### F23. Accessibility: icon grid, inspector, Quick Look

The icon grid as a grid of items, the inspector's facts as label and value pairs, Quick Look as a
dialog named after the item, the progress row as a progress bar with its value.

- Tests: each node exists with its role and name; the progress node's value follows the operation.

### F24. One process, many windows

Files runs every window in one process with `run_surfaces`: Super+N, Open in New Window and a
second launch open a window in the running process. Each window keeps its own browser, tabs and
view; apps, icons, thumbnails and the search index are shared.

- Tests: Super+N adds a window to the same process; closing one window keeps the others; Super+Q
  closes all.
- Acceptance: Super+N to first frame under 100 ms (before and after numbers).

### F25. Shared operations, Undo across windows, progress on the bar

`Operations` and the undo history move from the view to the app: a move started in one window is
undone with Super+Z in another, and progress shows in every window. Copy progress also goes to the
bar icon through `com.canonical.Unity.LauncherEntry`; the terminal's sender moves to a shared lib
as its second consumer.

- Tests: a move in window A is undone from window B; a running copy publishes `progress` and
  `progress-visible` on the bus and clears them when done.

### F26. `org.freedesktop.FileManager1` and single instance

Files owns `org.freedesktop.FileManager1` on the session bus: `ShowFolders` opens windows,
`ShowItems` opens the parent and selects the items, `ShowItemProperties` selects them with the
inspector shown. A D-Bus activation file in `data/` starts Files when it is not running; a second
`unbound-files <path>` hands its path to the running instance. Activity Monitor's "Show in Files"
then selects the executable.

- Tests (hermetic bus): `ShowItems` with two files in one folder opens one window with both
  selected; a second launch does not start a second process.

## 7. Polish checklist (every slice)

- Every new action is in a menu with its shortcut, in the context menu where it applies, and
  reachable without a pointer. New shortcuts are checked against the compositor's table in
  `libs/keyboard/src/shortcuts.rs` (Alt+Super+digits, arrows, `=`, `-` and `0` are taken).
- Every new surface has hover, pressed, focused, disabled and drag-over states, light and dark, at
  1× and 1.5×.
- Values come from Datum tokens: radii from the concentric rule, text roles, `ink` tiers, the
  accent only for selection and focus, semantic colours only for status.
- Numbers in columns use tabular figures; sizes and dates come from `format_size` and the new
  relative `format_date`.
- Nothing new blocks the UI thread; anything that touches the disk runs on a worker with a
  generation check, like `Loader`.
- Long and right-to-left names truncate in the middle and keep the extension visible.
- Strings go through `unbound-l10n`; the message-id test covers every new file.

## 8. Needs from other crates

- `toolkit/widgets`: possibly lazy cells or a row source for `Table` (F4).
- `testing/bench`: the big-folder actions (F1).
- `apps/terminal` and a shared lib: the LauncherEntry sender moves out of `apps/terminal` (F25).
- `apps/monitor`: switch "Show in Files" to `ShowItems` once F26 lands.
- `apps/file-chooser`: picks up the preview column, list expansion and empty states through the
  shared code where it already shares it; its own polish is another task.

## Sources

- F1. Apple, Change how folders are displayed in the Finder on Mac:
  https://support.apple.com/guide/mac-help/change-how-folders-are-displayed-mchldaafb302/mac
- F2. Apple, Mac keyboard shortcuts (Finder and system): https://support.apple.com/en-us/102650
- F3. Apple, View and edit files with Quick Look on Mac:
  https://support.apple.com/guide/mac-help/view-and-edit-files-with-quick-look-mh14119/mac
- F4. Apple, Use the Preview pane in the Finder on Mac:
  https://support.apple.com/guide/mac-help/use-the-preview-pane-mchl1e4644c2/mac
- F5. Apple, Rename files, folders and disks on Mac:
  https://support.apple.com/guide/mac-help/rename-files-folders-and-disks-mchlp1144/mac
- F6. Apple, Narrow your search results in the Finder on Mac:
  https://support.apple.com/guide/mac-help/search-for-files-on-mac-mh15155/mac
- W1. Microsoft, Inside this update (File Explorer archives and tabs):
  https://support.microsoft.com/en-us/windows/deployment/updates-lifecycle/inside-this-update
- W2. Microsoft, What's new in Windows 11, version 25H2 (tabs restored at sign-in):
  https://learn.microsoft.com/en-us/windows/whats-new/whats-new-windows-11-version-25h2
- G1. GNOME Help, Search for files: https://help.gnome.org/users/gnome-help/stable/files-search.html.en
- G2. GNOME Help, Rename a file or folder: https://help.gnome.org/users/gnome-help/stable/files-rename.html.en
- K1. KDE, The Dolphin Handbook: https://docs.kde.org/stable_kf6/en/dolphin/dolphin/index.html
- X1. freedesktop.org, File Manager D-Bus interface (not fetched: the site refused automated
  access; the three methods are as implemented by Nautilus and Dolphin):
  https://www.freedesktop.org/wiki/Specifications/file-manager-interface/
- The proposal's own sources: [`../design/features/files.md`](../design/features/files.md#sources).
