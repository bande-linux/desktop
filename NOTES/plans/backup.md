# Backup

A native Time Machine equivalent. Package `bande-backup` (`crates/services/backup`), Settings ▸
Backup, Files, recovery.

## What the user experiences
Settings ▸ Backup: choose a disk, type a password for it (asked once; needed only to restore on
another computer), done. Hourly versions from then on whenever the disk is there, kept hourly for
24 h, daily for 30 days, weekly until the disk fills. Oldest go first. Silent unless something is
wrong: no backup for 14 days, disk too small, damage found, disk unreadable. Pause lasts a day at
most. Files shows past versions of a file and past states of a folder and copies them back.
Recovery restores the whole system.

## Engine: restic, the Debian package
Measured on a builder (`torture` runs: 800 MB of random data; kill -9, power cut through dm-flakey
dropping writes, I/O errors through dm-error, flipped bytes in three packs, full disk):

| | restic 0.19 | borg 1.4 | kopia 0.23 | rustic 0.11 |
|---|---|---|---|---|
| Debian package | yes | yes (Python) | no | no |
| kill -9, power cut, unplug: repo intact, restores identical | yes | yes | yes | yes |
| Corrupt packs: check finds it | yes | yes | test missed its files | yes |
| Corrupt packs: restore of the rest | 397 of 400 files, errors named | stops at 124 of 400 | test missed its files | **aborts, 0 of 400** |
| Repair then new backup heals | yes (`repair packs`, `repair snapshots --forget`) | yes (`check --repair`) | not tested | not by `repair index` |
| Full disk: repo intact, prune frees space at 100% | yes | yes | yes | not tested |
| Encryption built in, no setup | yes | repokey | yes | yes |
| Disk filesystem can be anything (metadata in the repo) | yes | yes | yes | yes |
| Network later | sftp, rest, s3 | ssh only | many | many |

restic restores the most from a damaged disk, needs no runtime, has the widest use, and is packaged by
Debian. borg needs Python and only reaches networks over ssh. kopia and rustic are not in Debian.
btrfs send/receive was rejected before testing: needs btrfs on both ends,
no encryption without LUKS on the target, nothing works from ext4, a damaged stream is lost
whole. A format of our own has no justification: the format is the hard part, restic's is
documented, and any machine with restic can read a disk without Bande.

rustic_core as a library would save a process, but it panicked on corruption and calls itself beta.
So `restic` runs as a child process: exit codes and `--json` are its stable interface.

Measured behaviours the design rests on:
- An interrupted backup leaves packs without an index, so the next run stores everything again
  (1100 MB for 800). `restic repair index` first picks them up (802 MB). So pause, disconnect and
  crash all resume, by a marker in the state.
- `forget --prune` does not prune when nothing was forgotten; orphans need a plain `prune`.
- restic retries a dead disk for many minutes. The service watches for the disk on each progress
  line and stops it.
- Stale locks after a crash block `check`. The service owns the repository, so it clears them.
- `prune` works at 100% full (a lock file fits), so no reserve file. Backups stop at
  min(512 MiB, 10%) free, then the oldest quarter of versions go and the backup goes on.
- `restic mount` applies the snapshot's owners and modes through the kernel: a user reads their
  own old files, not another's, and not `/etc/shadow`. It is read-only. stat across all versions
  of a file takes milliseconds. So Files browses a plain folder and copies as the user, with no
  privileged restore path at all.

## Design
- **System service** (`bande-backupd`, root, `org.bande.Backup` on the system bus): whole-system
  restore needs all files, so it cannot be per user. Started by a timer that exists only once a disk
  is chosen, by udev when a disk or mains power arrives, or by D-Bus. Exits when idle; no wakeups.
- **Keys:** a random machine key in `/var/lib/bande-backup/key` runs the backups, unattended. The
  person's password is a second restic key, for restoring when the machine is gone. A key on the
  machine alone would make the backup useless in the case it exists for.
- **Disk:** any non-startup filesystem, kept as it is; a `BandeBackups/<machine-id>` folder on it. Mounted under
  `/run/bande-backup/disk` only during work. Eject during a backup asks the service to let go: the backup stops
  cleanly and resumes next time. A blank or foreign disk can be erased from Settings after a warning naming
  the whole drive and what is on it: UDisks makes a GPT disk with one ext4 partition.
- **Network:** `smb://` and `nfs://` are mounted by the service with the same soft options as Connect to Server,
  because there may be no session and the share password can't come from a person's keychain; the password
  sits in `/var/lib/bande-backup/share-password` (0400). restic's rest server is used directly (`rest:https://...`);
  `sftp:` isn't offered, as it has no unattended sign-in for root. A server that goes silent: a TCP probe on each
  progress line stops restic, a forced unmount clears the mount, and it counts as a disk that is away.
- **Local snapshots:** while the destination is away, paused or waiting for power, the hourly run on a btrfs install
  takes a read-only snapshot of `/home` in `/.snapshots/home/local-<time>`, keeping 24 hours, and deletes them early
  when `/home` is under 5% free. Files lists them with the backups, bound read-only under
  `/run/bande-backup-local/versions`.
- **What:** every local filesystem except caches, trash, temp and journals, one-file-system. Excludes
  `CACHEDIR.TAG` folders.
- **Snapshots:** on btrfs, a read-only snapshot in `/.snapshots/home/` (the part of the layout that
  `fix-update-safety` gives this gap) is the source, seen at the same paths in a private mount
  namespace, so a backup is one point in time. ext4 reads live files.
- **Integrity:** daily `check` of 1/30 of the data, so everything is read monthly. Damage: repair,
  tell the person, the next backup stores what's still on the computer again.
- **Alerts:** `bande-backup-alerts`, a small session service, notifies when the service reports a
  problem. System services cannot show notifications.
- **Files:** Versions of a file, and Browse Backups for a folder, over `/run/bande-backup-browse/versions/<time>/...`.
- **Recovery:** `bande-backup-restore`, no bus, for the recovery environment: lists the backups on a
  disk, restores one onto a mounted new root, keeping the new root's fstab, crypttab and boot files.
  `fix-first-run` calls it.

## Left to do
- `fix-first-run` calls `bande-backup-restore` from Recovery and adds the package to the image.
