folder = Folder
trash = Trash
sidebar-favourites = Favourites
sidebar-locations = Locations
empty-folder = Empty folder
no-selection = No selection
item-count = { $count ->
    [one] { $count } item
   *[other] { $count } items
}
selected-count = { $selected } of { $count } selected
status-free = { $items }, { $free } available

size-bytes = { $count ->
    [0] Zero bytes
    [one] { $count } byte
   *[other] { $count } bytes
}
size-kilobytes = { $value } KB
size-megabytes = { $value } MB
size-gigabytes = { $value } GB
size-terabytes = { $value } TB

info-size = Size
info-modified = Modified
info-where = Where
info-original-location = Put back to
info-deleted = Deleted
info-permissions = Permissions
info-link = Points to
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
rename-exists = The name “{ $name }” is already taken. Please choose a different name.
error-operation = The operation couldn’t be completed: { $error }
error-item = “{ $name }” couldn’t be handled: { $error }{ $others ->
    [0] {""}
   *[other] {" "}({ $others } more)
}
error-no-app = There is no app to open “{ $name }” ({ $kind }).
error-default-app = The default app couldn’t be changed: { $error }
