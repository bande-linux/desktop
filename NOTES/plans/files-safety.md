# Files data safety

What the user gets: a copy or move either finishes or leaves things as they were, whatever happens (power, pulled disk, Quit, Log Out), and every file on an old disk can be opened.

## Design
- Every copied item (a file, or a whole folder) is written as `.bande-partial-<pid>-<n>` in the destination, synced, then renamed (no-replace). Sync: `fsync` for one file; `syncfs` once for a tree, since per-file `fsync` costs minutes on large trees (4 ms each measured on the mini). Flushing every 64 MiB keeps progress honest and finds a dead disk early.
- A move deletes its source only after the rename and the folder sync.
- Replace trashes the old item only when the new one is ready.
- An `flock` held by the writer tells a running copy from a crash leftover; listing a folder removes leftovers. O_TMPFILE would leave nothing behind, but FAT and exFAT, the usual USB disks, lack it, so one path for all.
- Quit, Log Out and closing the last window ask while operations run; closing another window hands them over. Log Out already asks apps through `should_close`.
- Paths stay bytes from listing to launch. `%f` passes raw bytes; a `bande-app` turns a non-UTF-8 argument into a `file://` URI, since its arguments are text.

## Stages
1. bande-fs, bande-xdg, bande-app and Files (this branch).
2. Apps: Archive takes URIs; Mail and Calendar stop writing exports in place and say when it fails; a refused Mail change is shown.
