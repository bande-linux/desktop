# Update safety

An update can never leave the computer unusable. Branch `fix-update-safety`; audit gap 1.

## What the user experiences
- System updates install in the background on a copy of the system. The running system is not
  touched, so a power cut or a failed install changes nothing. When it is ready: "Restart to
  Update". The restart takes as long as any restart.
- If the new version cannot reach the login window, the computer goes back to the previous version
  by itself after at most three tries, and says so: "The update couldn't start. Your computer went
  back to the version from Tuesday."
- Software Update has "Check for updates": When available, Every day, Every 7 days, Every 31 days.
  Between checks nothing is fetched or installed, even at startup, and Check Now always works. The
  pane shows the date of the last check and says that longer intervals delay security fixes.
- Settings ▸ Software Update ▸ Previous Versions lists versions by date and what changed, with one
  Restore button. Recovery offers the same list. Files in `/home` are never affected. Passwords,
  accounts, Wi-Fi and paired Bluetooth devices stay as they are now.

## Disk layout (new installs, shared with fix-first-run, fix-security, fix-backup)
GPT: ESP (FAT32, 1 GB), Recovery (6 GB, read-only, fix-first-run), Root (LUKS2, fix-security)
holding one btrfs. The mini stays on ext4 and is not reinstalled.

| Subvolume at the btrfs top level | Mounted at | Rolled back |
|---|---|---|
| `@system/<id>` (`@system` is a plain directory) | `/` | yes: this is a system version |
| `@home` | `/home` | never |
| `@var-log` | `/var/log` | never, the log of a failed boot must survive |
| `@var-cache` | `/var/cache` | never, not worth snapshotting |
| `@var-tmp` | `/var/tmp` | never |
| `@snapshots` | `/.snapshots` (0700) | never; `home/` inside belongs to fix-backup |

Root is chosen only on the kernel command line:
`rootflags=subvol=@system/<id>,noatime,compress=zstd:1`. So `/etc/kernel/cmdline` has no
`rootflags`, and the `/` line in fstab has no `subvol=`:

```
UUID=<fs> /           btrfs noatime,compress=zstd:1 0 0
UUID=<fs> /home       btrfs subvol=@home,noatime,compress=zstd:1 0 0
UUID=<fs> /var/log    btrfs subvol=@var-log,noatime,compress=zstd:1 0 0
UUID=<fs> /var/cache  btrfs subvol=@var-cache,noatime,compress=zstd:1 0 0
UUID=<fs> /var/tmp    btrfs subvol=@var-tmp,noatime,compress=zstd:1 0 0
UUID=<fs> /.snapshots btrfs subvol=@snapshots,noatime,compress=zstd:1 0 0
```

The first version is `@system/1`, with entry `loader/entries/bande-1.conf` and no tries: a first
boot has nothing to fall back to. Subvolumes can be added later without a reinstall, so anything
else waits until something needs it.

## Design
**Versions, not a mutable root with rollback.** Every `@system/<id>` is a writable, bootable
subvolume with `@system/<id>.info` next to it (created, why, what changed).

**System updates** (`Class::Boot`, `Class::Offline`): `bande-system prepare` clones the running
version, runs apt in a chroot of the clone with services blocked, and on success writes its kernel
and initrd to the ESP and then its entry. The entry is the single atomic step (a rename on the
ESP). Any failure before that deletes the clone. It replaces PackageKit's offline mode on this
layout; a machine without it, such as the mini on ext4, keeps PackageKit's offline mode.

**Everything else** (`Class::Live`, a terminal `apt`): apt's `DPkg::Pre-Invoke`, which runs once per
transaction, keeps an unchanged copy as a version and re-arms boot counting on the running one,
before dpkg changes anything. PackageKit goes through it too. A plain `dpkg -i` does not, since
dpkg's own hook also fires between the steps of a transaction, and a half-done copy must never
become the fallback. A change on the running system while a prepared update waits discards that
update, which is prepared again.

**Boot.** systemd's Automatic Boot Assessment, no own mechanism: `bande-<rank>+3.conf`, ranks only
go up, systemd-boot picks the newest with tries left. `boot-complete.target` needs
`bande-boot-health.service`, which waits for a datagram the compositor sends to
`/run/bande/login-ready` once the login window or the desktop is really shown, and reboots after
5 minutes. Only a counted boot runs it. On a counted boot emergency and rescue mode reboot at once
and `panic=10` covers the kernel and initramfs, so no failure waits for someone at a console. The
running system's `systemd-bless-boot` removes the counter. U-Boot and systemd-boot rename the entry
on the FAT ESP; EFI variables are not needed for writing.

**ESP.** Kernels and initrds are named by content hash (`/bande/<hash>/`), so versions share them
and an initrd rebuilt for one version never changes another. `layout=bande` in
`/etc/kernel/install.conf` replaces kernel-install's own entry writing. `unboot/boot.bin` (m1n1,
U-Boot, devicetrees) is shared by all versions and cannot be rolled back this way: a bad one needs
macOS recovery or the tether. An older kernel can also run with a newer devicetree until Restore
rewrites `boot.bin` from the version it restores.

**State that follows you.** On the first boot of a version, before anything else starts, it copies
from the version the computer last ran: human accounts (UID 1000 and up, with shadow entries and
group membership, merged over the version's system users), `/etc/NetworkManager/system-connections`,
`/var/lib/bluetooth`, `/var/lib/AccountsService`, ssh host keys, hostname, time zone, locale,
machine id. It is one list in one place, so a prepared update also gets a password changed just
before the restart.

**Restore** creates a new version from the chosen one and boots it, so a restore can itself be
undone. The tool is shell plus btrfs-progs in `bande-scripts`; Settings calls it through a system
service, and the recovery OS (fix-first-run) calls it from its menu.

**How often.** The daily user timer `bande-update-check.timer` stays the one timer. It reminds about
a pending restart at every tick and only checks when the interval has passed since PackageKit's
last refresh, the way `apt.systemd.daily` uses its own daily timer and stamps. Debian's
`apt-daily.timer` fetches nothing while `APT::Periodic` is unset, as it is here. Testing has no
security suite to ask cheaply (its `testing-security` is empty and fixes arrive by migration), so
security updates follow the same interval, and the setting says so.

**Cleanup.** Keep the running version, the last good one and the newest few that fit: fewer when
free space is short. Failed versions go at the next update.

## Why this over the alternatives
- **`@` plus snapper, restoring by renaming:** the update still changes the running root, so a power
  cut leaves a half-installed system that may boot. macOS-style atomic activation is the bar, and
  installing before the restart makes the restart short too.
- **Booting read-only snapshots:** services fail on a read-only root, and it needs a second "promote"
  step. Writable versions make fallback an ordinary boot.
- **Own boot counter or GRUB:** systemd-boot already does this and is the boot chain we have.
- **A/B image updates (systemd-sysupdate):** Bande follows Debian packages, not signed images.

## Known limits
- Tools that make nested btrfs subvolumes under `/` (Docker's btrfs driver) do not carry over to a
  new version. Give them a top-level subvolume when that becomes real.
- `/etc` edits made after a prepared update was built, and any on a restored version, are not kept,
  except the list above.

## Stages
1. Boot assessment: entries with tries, health check, deadline. Proved in QEMU with U-Boot and
   systemd-boot.
2. Layout and versions: `bande-system`, the btrfs image, rollback proved in QEMU, 16K kernel.
3. Settings: prepared updates, Previous Versions, the went-back message.
4. Recovery: Restore in the recovery OS.
5. How often to check: the setting above.

m1n1 cannot run in QEMU. It takes no part in choosing an entry, so the mini's own check is one owner
step: install a new image and pull the plug during a boot.
