folder = Folder
trash = Trash
sidebar-favourites = Favourites
sidebar-locations = Locations
empty-folder = Empty Folder
trash-empty = The Trash is empty
recents-empty = Files you open appear here
no-results = No results for “{ $text }” in { $folder }
search-home = Search Home
no-permission = You don’t have permission to see the contents of “{ $folder }”.
no-selection = No selection
column-name = Name
search-placeholder = Search
column-modified = Date Modified
column-created = Date Created
column-size = Size
column-kind = Kind
item-count = { $count ->
    [one] { $count } item
   *[other] { $count } items
}
selected-count = { $selected } of { $count } selected
status-free = { $items }, { $free } available

info-name = Name & Extension
info-size = Size
info-modified = Modified
info-created = Created
more-info = More Info
kind-and-size = { $kind } · { $size }
info-where = Where
info-original-location = Put back to
info-deleted = Deleted
info-permissions = Permissions
info-link = Points to
info-dimensions = Dimensions
info-pixels = { $width } × { $height }
sharing-permissions = Sharing & Permissions
allow-running = Allow running as a program
access-you = You
access-group = Group
access-everyone = Everyone
access-read-write = Read & Write
access-read-only = Read only
access-none = No Access
error-permissions = The permissions couldn’t be changed: { $error }
folder-size = { $size } for { $count ->
    [one] { $count } item
   *[other] { $count } items
}
calculating = Calculating…
open-with = Open with
default-app = { $app } (default)
open = Open
use-for-all = Use for All

progress-copy = Copying { $done } of { $total } items
progress-move = Moving { $done } of { $total } items
progress-duplicate = Duplicating { $done } of { $total } items
progress-trash = Moving { $done } of { $total } items to the Trash
progress-put-back = Putting back { $done } of { $total } items
progress-rename = Renaming
progress-new-folder = Creating a folder
progress-undo = Undoing
progress-redo = Redoing
progress-erase = Deleting { $done } of { $total } items
progress-bytes = { $done } of { $total }

conflict-title = An item named “{ $name }” already exists in this location.
conflict-detail = Existing: { $existing }
    New: { $new }
apply-to-all = Apply to all
stop = Stop
skip = Skip
replace = Replace
keep-both = Keep Both
skip-all = Skip All
try-again = Try Again
ok = OK
internal-disk = Internal
failure-title = “{ $name }” couldn’t be { $action ->
    [copy] copied
    [move] moved
    [duplicate] duplicated
    [trash] moved to the Trash
    [put-back] put back
    [delete] deleted
   *[other] changed
}.
failure-permission = You don’t have permission.
failure-read-only = The volume is read-only.
failure-no-space = The disk is full.
failure-name-too-long = The name is too long.
failure-device-gone = The disk was disconnected.
failure-other = { $error }
no-space-item = “{ $name }” can’t be { $action ->
    [move] moved
    [duplicate] duplicated
   *[other] copied
} because “{ $volume }” needs { $size } more space.
no-space-items = The items can’t be { $action ->
    [move] moved
    [duplicate] duplicated
   *[other] copied
} because “{ $volume }” needs { $size } more space.
no-space-detail = Nothing was written. Make room on the disk and try again.

cancel = Cancel
empty-trash = Empty Trash
empty-trash-title = Are you sure you want to permanently erase the items in the Trash?
delete = Delete
delete-title = { $count ->
    [one] Delete this item immediately?
   *[other] Delete these { $count } items immediately?
}
cannot-undo = You can’t undo this action.

rename-empty = A name can’t be empty.
rename-slash = A name can’t contain “/”.
rename-reserved = The names “.” and “..” are reserved by the system.
extension-change-title = Are you sure you want to change the extension from “{ $from }” to “{ $to }”?
extension-add-title = Are you sure you want to add the extension “{ $extension }” to the end of the name?
extension-remove-title = Are you sure you want to remove the extension “{ $extension }”?
extension-detail = If you make this change, your document may open in a different app.
extension-keep = Keep { $extension }
extension-use = Use { $extension }
extension-add = Add
extension-dont-add = Don’t Add
extension-remove = Remove
extension-dont-remove = Keep
rename-exists = The name “{ $name }” is already taken. Please choose a different name.
error-operation = The operation couldn’t be completed: { $error }
error-item = “{ $name }” couldn’t be handled: { $error }{ $others ->
    [0] {""}
   *[other] {" "}({ $others } more)
}
go-to-title = Go to the folder:
go = Go
error-go-to = “{ $path }” can’t be found.
error-default-app = The default app couldn’t be changed: { $error }

menu-open-with = Open With
menu-open-with-other = Other…
open-with-title = Choose an app to open “{ $name }”:
open-with-items-title = Choose an app to open { $count } items:
always-open-with = Always Open With
menu-quick-look = Quick Look
menu-quick-look-full-screen = Quick Look in Full Screen
quick-look-position = { $at } of { $count }
quick-look-open-with = Open with { $app }
menu-get-info = Get Info
menu-show-in-enclosing-folder = Show in Enclosing Folder
menu-rename = Rename
menu-rename-items = Rename { $count } Items…
batch-title = Rename { $count } Items
batch-replace-text = Replace Text
batch-add-text = Add Text
batch-format-names = Format
batch-find = Find:
batch-replace-with = Replace with:
batch-text = Text:
batch-where = Where:
batch-after-name = after name
batch-before-name = before name
batch-format = Format:
batch-name-index = Name and Index
batch-name-counter = Name and Counter
batch-custom-format = Custom Format:
batch-start-at = Start numbers at:
batch-start-number = Start numbers at a whole number.
batch-twice = Two items would both be named “{ $name }”.
batch-example = { $old } → { $new }
menu-duplicate = Duplicate
menu-make-alias = Make Alias
menu-copy = Copy
menu-copy-path = Copy as Pathname
menu-copy-path-of = Copy “{ $name }” as Pathname
menu-copy-paths = Copy { $count } Items as Pathnames
menu-add-to-sidebar = Add to Sidebar
menu-undo-action = Undo { $action }
menu-redo-action = Redo { $action }
edit-copy = Copy of { $count ->
    [one] { $count } Item
   *[other] { $count } Items
}
edit-move = Move of { $count ->
    [one] { $count } Item
   *[other] { $count } Items
}
edit-duplicate = Duplicate of { $count ->
    [one] { $count } Item
   *[other] { $count } Items
}
edit-rename = Rename
edit-new-folder = New Folder
edit-new-document = New Document
edit-trash = Move to Trash
edit-put-back = Put Back
edit-compress = Compress
edit-extract = Extract
edit-make-alias = Make Alias
menu-cut = Cut
menu-move-to-trash = Move to Trash
menu-move-items-to-trash = Move { $count } Items to Trash
menu-put-back = Put Back
menu-delete-immediately = Delete Immediately…
menu-new-folder = New Folder
menu-paste = Paste
menu-sort-by = Sort By
menu-as-icons = as Icons
menu-as-columns = as Columns
menu-as-list = as List
menu-bigger-icons = Bigger Icons
menu-smaller-icons = Smaller Icons
menu-show-hidden-files = Show Hidden Files
menu-hide-hidden-files = Hide Hidden Files
menu-empty-trash = Empty Trash…
menu-remove-from-sidebar = Remove from Sidebar
menu-eject = Eject
eject-busy = The disk “{ $name }” wasn’t ejected because one or more programs may be using it.
eject-busy-detail = To eject the disk immediately, click Force Eject. Programs using it may lose unsaved changes.
eject-failed = The disk “{ $name }” couldn’t be ejected.
force-eject = Force Eject

app-name = Files
menu-file = File
menu-edit = Edit
menu-view = View
menu-go = Go
menu-open = Open
menu-find = Find
menu-undo = Undo
menu-redo = Redo
menu-move-item-here = Move Item Here
menu-select-all = Select All
menu-back = Back
menu-forward = Forward
menu-enclosing-folder = Enclosing Folder
menu-desktop = Desktop
menu-documents = Documents
menu-downloads = Downloads
menu-home = Home
menu-go-to-folder = Go to Folder…
menu-recent-folders = Recent Folders
menu-clear-menu = Clear Menu
icon-size = Icon size
menu-open-in-new-tab = Open in New Tab
menu-new-window = New Window
menu-new-tab = New Tab
menu-previous-tab = Show Previous Tab
menu-next-tab = Show Next Tab
recents = Recents
search-title = Searching “{ $folder }”
search-scope = Search:
search-scope-folder = “{ $folder }”
search-scope-home = Home
search-kind = Kind
search-date = Date Modified
kind-any = Any Kind
kind-folder = Folder
kind-alias = Alias
kind-document = Document
kind-image = Image
kind-audio = Audio
kind-video = Video
kind-archive = Archive
date-any-time = Any Time
date-today-filter = Today
date-last-week = Last 7 Days
date-last-month = Last 30 Days
date-this-year = This Year
column-where = Where
view-label = View
view-icons = Icons
view-list = List
view-columns = Columns
menu-compress = Compress “{ $name }”
menu-compress-items = Compress { $count } Items
progress-compress = Compressing { $done } of { $total } items
progress-extract = Extracting { $done } of { $total } items
menu-compress-selection = Compress
menu-open-in-terminal = Open in Terminal
menu-new-document = New Document
menu-status-bar = Show or Hide Status Bar
menu-restore-sidebar = Restore Default Sidebar
settings-title = Files Settings
settings-general = General
settings-new-window = New windows show
settings-home = Home
settings-current-folder = Current Folder
settings-last-folder = Last Folder
settings-default-view = Default view
settings-show-hidden = Show hidden files
settings-desktop-disks = Show external disks on the desktop
settings-spring-loading = Spring-loaded folders
settings-spring-delay = Spring-loading delay
settings-folders-first = Keep folders on top when sorting by name
settings-sidebar = Show in Sidebar
settings-external-disks = External disks
settings-advanced = Advanced
settings-show-extensions = Show all filename extensions
settings-warn-extension-change = Show warning before changing an extension
settings-warn-empty-trash = Show warning before emptying the Trash
settings-purge-trash = Remove items from the Trash after 30 days
settings-search-in = Search in
