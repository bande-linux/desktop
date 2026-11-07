# Search that knows contents

## What the user gets
Type a phrase in Search or Files and get the document holding it, the mail, the event, the contact, a second or two after it was saved. Results are there the moment the shell restarts or the session starts, since the index lives on disk. The machine never feels it: indexing runs only on idle CPU and idle disk, and with nothing changing the service does not wake at all.

## Design
- **`bande-indexer`**, a session service on `org.bande.Search` (`Find`, `Donate`, `Forget`). One index for Search and Files; each used to crawl the home folder on its own, from nothing.
- **One thread owns the state, one does the slow work.** A normal-priority thread keeps the name tree and answers queries. A `SCHED_IDLE`, idle-I/O thread walks folders, reads files and owns the index writer, so a query never waits behind indexing, even on a full machine.
- **Names and contents.** File names stay in the existing in-memory tree with the fuzzy matcher; ids are stable slots in it. Contents go in `tantivy` under that id, so renaming a folder rewrites one record and re-reads nothing. The tree is saved in the index and restored at start.
- **Keeping current.** inotify per folder, changes settled for 300 ms and matched by inode so moves stay moves. First save 2 s after a change, then at most every 10 s. `IN_Q_OVERFLOW`, and every start, re-check the tree against the disk by name, inode, time and size; Linux keeps no change journal, so this is the only way to see what happened while nothing listened.
- **Importers.** Plain text in the indexer. PDF, Word, PowerPoint, Excel, OpenDocument, RTF, HTML and EPUB in a child per file: memory limit, 15 s limit, and seccomp allowing only compute once the file has been read, because these parsers now run on any downloaded file with nobody asking.
- **What is left out.** Hidden folders, caches (`CACHEDIR.TAG`, `node_modules`), Settings ▸ Search's excluded folders (also rewritten out of the index files at once), anything behind another mount, and fscrypt-encrypted folders. So an encrypted volume's contents never land in an index outside it. The index stays on the home folder's own mount, mode 0700.
- **Mail, Calendar, contacts.** Apps donate items, as Core Spotlight does: id, kind, title, detail, what to open, text. The app owns its store and its schema; bodies are indexed but not stored, and no text is copied out.
- **Low memory.** Importing pauses below 512 MiB available; the unit sits in `background.slice` with low weights and `MemoryHigh`.

## Why this over alternatives
Measured on 106,692 files, 1.6 GB of text (the kernel tree), top 40 by relevance:

| | tantivy 0.25 | SQLite FTS5 3.53 (`detail=full`, prefix 2,3) |
|---|---|---|
| Index size | 391 MiB | 983 MiB after optimize, 1,259 before |
| CPU to build | 29 s | 49 s + 21 s optimize |
| Common word (`the`) | 0.5 ms | 69 ms |
| Prefix `s*` | 0.3 ms (first 200 words) | 1,260 ms |
| Prefix `sch*` | 0.1 ms | 15 ms |
| Phrase | 1.3 ms | 6 ms |
| One update and save | 41 ms | 1.8 ms |

Search runs on each keystroke, so query speed decides, and saves are batched anyway. tantivy is built without its zstd C code; it adds about 50 pure Rust crates and only this service links it.

- **fanotify**: a whole-filesystem mark needs `CAP_SYS_ADMIN`, so a privileged helper reading everyone's file events to spare one user a watch per folder. Unprivileged fanotify offers only per-inode marks, the same as inotify. Not worth the attack surface.
- **Apps' stores read directly** (the indexer opens Mail's SQLite file): ties a desktop service to each app's schema, across repositories, and breaks silently.
- **Asking apps at query time** (GNOME's search providers): starts Mail on a keystroke, and is only as fast as the slowest app.

## Stages
1. Service, files and contents, Search and Files on it.
2. `Donate` shown in Search: groups, Settings categories, opening.
3. Mail and Calendar donate (apps repo); senders and attendees as contacts.

Later, if wanted: removable volumes with an index on the volume itself, as Spotlight does; Files walks them for names until then.
