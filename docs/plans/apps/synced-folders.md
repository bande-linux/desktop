# Synced folders

Domain: [files](../../architecture/intents/domains/files.md). Packages A11a, A11b, A11c. Size L
each.

A folder here and one on a server the person has are kept the same. It is an ordinary folder: every
app sees ordinary files, it works offline, and nothing about it needs learning. WebDAV and SFTP
first; a backend is one trait, so others (SMB, a disk, object storage) can be added without touching
the rest.

**There is.** Servers mounted by Files (`bande-automount`), saved sign-ins, the Connect sheet,
folder watching (`bande-watch`), file operations with Trash, Files ▸ Settings. No synchronisation.
OpenSSH's client is packaged and speaks SFTP over `ssh -s`.

## For the person

- **Set up** in Files ▸ Settings ▸ Synced Folders ▸ Add…, one sheet: the server (a saved one, or an
  address with the same sign-in as Connect to Server), the folder there, the folder here (named
  after it in Home at first). Before anything moves it says what will happen: "212 files here, 180
  there: 40 are copied up, 8 down, 172 look the same, none are deleted." The first sync only adds:
  what looks the same is verified by content, and different content with one name is kept as both.
- **The list** has each folder with one line of state ("Up to date", "Copying 3 of 40", "Waiting for
  the server since 14:02", "2 need you") and, for the selected one, Pause, Sync Now, Options… (when:
  automatically, not on metered networks, or only when asked; folders left out) and Stop Syncing…,
  which keeps the files on both sides.
- **In Files** a synced folder has a small mark on its icon, and its window a quiet line at the
  bottom with the state. There are no marks on files, no columns and **no context menu entries**.
  Where something needs the person, the line says so and opens the list of those files.
- **Conflicts** are kept as both: the server's keeps the name, this computer's becomes "Report (from
  laptop, 25 Nov 14.02).odt". The list shows each pair with Open Both, Keep This and Keep Other;
  what is not kept goes to Trash, and only if both are still the versions shown.
- **Deleted** files go to Trash here, and on the server to a hidden folder for 30 days.
- **Notifications** only when the person must do something: sign in again, a full server, a brake
  (below), a conflict. A server that is away is not a problem and says nothing.

## How it stays right

One rule decides everything: **nothing is overwritten or deleted that was not seen, in that version,
by the last sync.**

- **Three states.** A database per pair holds what both sides last agreed on: path, kind, size,
  time, digest, and the server's version. A sync compares here, there and that, in one pure function
  from three listings to a list of actions. Changed on one side: copied. On both, differently: kept
  as both. Edited here and deleted there, or the other way: the edit wins and the file comes back.
- **Listings are the truth, watching is a hint.** Local changes are noticed by `bande-watch` and
  settled for three seconds; folders are watched before they are listed; a full look happens at
  start, after an overflow or resume, and daily. When watches run out the pair looks every ten
  minutes and says so once. Only a name absent from a completed listing of its parent is deleted. A
  path that became a link, special file, mount point, left out or unreadable is suspended with
  everything under it and listed; leaving a folder out drops its records and removes nothing. A
  folder that could not be listed suspends only itself.
- **Changed or not.** A local file is unchanged only if inode, size, modification and change time
  (nanoseconds) all match. On SFTP size and time decide; a server time within two seconds of the
  listing, by the server's clock, is compared by content, and every file is read again over 30 days
  at idle.
- **The brake.** Counted over a rolling hour per side: more than 20 deletions that are a tenth of
  the folder, more than 500, or everything (two or more files); more than 100 replaced files that
  are a third of it, or any 20 whose time goes backwards. It offers Continue, Put Back (from the
  other side) and Show Files. A missing folder, one on an unmounted disk, another folder at the same
  place, or a server folder without its marker (`.bande-sync/id`) stops the pair; none is read as
  "all was deleted".
- **Local replacement** is `renameat2(RENAME_EXCHANGE)`, a new file `RENAME_NOREPLACE`, both from a
  checked and flushed temporary file. The displaced file is then checked by identity, size, time
  and, if in doubt, digest: equal to what was seen, it is removed; otherwise it becomes a conflict
  copy. Writers are detected with a read lease before and after, and a busy file waits. Where the
  filesystem lacks these, the old file is kept for that replacement. The existing `rename_new`,
  which may fall back to a plain rename, is not used.
- **WebDAV writes** are one `PUT` to the target itself, with `If-Match` the known strong ETag or
  `If-None-Match: *`; no temporary name and no `MOVE` over a file. A server found at set-up to
  ignore conditions, or with weak ETags only, is treated like SFTP. An upload that was cut off is
  done again before its server version is believed.
- **Without conditions** (SFTP, such servers), Bande computers take turns: a lock folder
  `.bande-sync/lock`, made atomically and renewed, is held while a batch is put in place, and a lost
  or stale lock stops the batch. Just before replacing, the version there is kept in
  `.bande-sync/replaced` (`hardlink@openssh.com`, or a copy) and compared; if it is not the known
  one it becomes a conflict. A new file is put in place with a rename that refuses an existing name.
  A server without atomic replace is add-only: edits there are kept as both. The gap left is another
  program writing between those two requests; set-up says so.
- **Versions.** The version recorded is the one in the write's own answer. Without it the file is
  read back and its digest compared; on a difference only our upload is recorded and the server's is
  a new change. Every write's intended digest is stored first, so a server version with that content
  is ours even when the answer was lost.
- **Folders are never removed as a whole**: seen files go one by one, then the folder if it is
  empty. Where WebDAV cannot ask that, it is moved to the server's deleted folder, listed there, and
  anything unseen is moved back. A folder with a new, changed or left-out item stays. A file in
  place of a folder, or the other way, is a removal and a creation under the same rule; unseen
  content on either side keeps both.
- **Moves** are recognised only if the old path is gone and the new one is the same inode with
  unchanged size and time, or the same digest; paths that still exist are edits. On the server a
  move needs a stable id, otherwise it is a removal and a copy. Case-only renames go through a
  temporary name where the server ignores case.
- **Conflict copies** are uploaded with no-replace, numbered if taken, sync as ordinary files, and
  are not made again for the same two digests.
- **Interrupted anywhere** (crash, power, network, logout), the next run starts from the three
  states again; an action is done only when seen on both sides. A full disk, a quota or one
  unacceptable file holds that file, not the pair.
- **Names** the other side cannot hold are left out and listed, never renamed silently. Links,
  sockets and devices are left out; times and, over SFTP, the executable bit are kept.
- **Left out always**: partial downloads, lock and swap files, `.bande-sync`, and what the person
  left out. SQLite files with a `-wal` or `-journal` beside them wait until it is gone. A folder
  with `.git` gets one sentence at set-up recommending it be left out. Set-up refuses pairs inside
  pairs, network and FUSE folders, Trash and the mount of the same server; a pair does not cross
  mounts.
- **The server's `.bande-sync`** (id, lock, replaced, deleted) is made with no-replace, and who
  loses the race takes the winner's id. Each computer prunes only what it put there, after 30 days
  by the server's clock, earliest first when the server is full.
- **Two computers** need not know each other: each syncs with the server by the same rule.

## How it is built

| Part | What |
| :--- | :--- |
| `crates/libs/sync` | the pure comparison, the database, the runner, and `trait Backend` (`capabilities`, `list`, `stat`, `read`, `write` with a condition, `rename`, `remove`, `make_folder`); a folder on this computer is the first backend and the test one |
| SFTP | the protocol over `ssh -s <host> sftp` as a child, so keys, agent and `~/.ssh/config` (a `ProxyCommand` is the person's own) are OpenSSH's, with `ClearAllForwardings`, no `LocalCommand`, `RemoteCommand`, terminal or multiplexing, `StrictHostKeyChecking=yes` and keep-alives; a host key is accepted only in the sheet. Automount's askpass moves to a library and answers only this user and host's password question. The connection is reused. No options come from the address |
| WebDAV | the sign-in and multistatus code Calendar has, in `crates/libs/dav`, with streamed bodies and stall instead of total timeouts; `PROPFIND` depth 1; TLS always, except to this computer |
| `crates/services/sync` | `bande-sync`, a user service that runs while a pair is on: at low I/O priority, one pair at a time and four transfers, pausing on low battery and metered networks as asked; Files is its client over `org.bande.Sync`. Automatic means five seconds after a change here, and the server at the larger of five minutes and twenty times the last listing, and after network or wake. Subtrees are skipped by folder ETag only where set-up proved that a nested change alters every ancestor's. It talks to nothing the person did not set up |
| Passwords | in the keyring under its own identity, the pair with its own address; a locked keyring means waiting without a question, and Sync Now may ask |
| Files | Settings ▸ Synced Folders, the mark, the line, the list; a removed or moved folder asks whether to stop or follow |
| Package | `bande-sync`, `Suggests` of `bande-files`; without it the settings page offers to install it |

Search, backups, sharing and apps need nothing: it is a folder. For agents and routines, writing in
a synced folder is `send`.

## Done when

- **A11a**: a simulator runs the engine against two fake sides and two computers with random edits,
  renames, deletions, clock skew, lost answers, full disks, crashes at every step, other programs
  writing, non-atomic `PUT`, missing, weak and changing ETags, backups restored on either side, and
  links or mounts appearing: in 10 million steps no content that existed is lost (Trash, replaced
  and conflict copies count, and only the named gap is excluded), all sides agree once quiet, and a
  second run does nothing. Every failure found is kept as a test.
- **A11b**: the same scenarios pass against real `sftp-server` and two unlike WebDAV servers, with
  the connection cut at random bytes; 100,000 files with nothing changed cost under 10 seconds
  against a local server and no writes.
- **A11c**: someone who has never seen it sets up a folder in under a minute; a week on two
  computers with suspend, pulled plugs and a moving laptop loses nothing; idle it does not wake
  between checks and stays under 15 MB; every state is readable by a screen reader, and no menu
  gained an entry.

Not now: syncing between computers without a server, files that are only on the server until opened,
encryption on the server, per-network choice, showing what was deleted on the server, and version
history beyond its 30 days.
