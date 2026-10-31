untitled-folder = untitled folder
copy-name = { $name } copy
copy-name-numbered = { $name } copy { $number }

size-bytes = { $count ->
    [0] Zero bytes
    [one] { $count } byte
   *[other] { $count } bytes
}
size-kilobytes = { $value } KB
size-megabytes = { $value } MB
size-gigabytes = { $value } GB
size-terabytes = { $value } TB
archive-name = Archive
date-today = Today, { $time }
date-yesterday = Yesterday, { $time }
error-no-name = The item has no name.
error-no-folder = The item is not in a folder.
error-trash-unusable = The trash on this disk can’t be used.
error-trash-no-free-name = The trash has no free name for the item.
error-across-mount = The folder contains another disk, so it can’t be deleted.
error-already-in-trash = The item is already in the trash.
error-name-taken = An item with that name already exists.
error-replace-own-folder = An item can’t replace the folder that contains it.
error-copy-into-itself = A folder can’t be copied into itself.
error-move-into-itself = A folder can’t be moved into itself.
error-special-file = Special files can’t be copied.
error-not-an-archive = The file is not an archive.
error-archive-through-link = The archive tries to write through a link.
error-no-7-zip = No program to open 7-Zip archives is installed.
error-archive-program = { $program } couldn’t open the archive.
error-not-found = The item can’t be found.
error-permission-denied = You don’t have permission.
error-storage-full = There isn’t enough space on the disk.
error-read-only = The disk is read-only.
error-folder-not-empty = The folder isn’t empty.
error-too-large = The file is too large for this disk.
error-busy = The item is in use.
error-invalid-name = The name isn’t valid here.
