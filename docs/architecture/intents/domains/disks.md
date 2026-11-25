# Disks

Part of [Bande Intents](../../intents.md). Things `disks.disk`, `disks.volume`. Owner: the Disks
app; the Terminal app for `terminal`. Work package D9.

| Thing | Properties (* foreign, ~ name) | In `find` |
| :--- | :--- | :--- |
| `disks.disk` | name~ text, size integer, connection `usb` `sata` `nvme` `card` `other`, removable, protected bool, health `good` `failing` `unknown`, temperature number | all |
| `disks.volume` | name~ text, disk ref, size, free integer, format `ext4` `exfat` `fat32` `ntfs` `other`, connected, encrypted, resizable, protected bool, formats list<enum> | all |

| Intent | Parameters | Effects | Undo | Code | P |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `disks.find_disk`, `disks.find_volume` | `text` | `read` |  | app | A |
| `disks.get` | `things`, `properties` | `read` |  | part: health is in `bande-volumes`, the rest in the app's model | A |
| `disks.disconnect_disk` | `disk` (eject) | `disrupt` |  | ready | A |
| `disks.connect_volume` | `volume`; an encrypted one is asked by its owner | `disrupt`; a system disk `critical` | `disks.disconnect_volume` | app (`jobs.rs`) | A |
| `disks.erase_disk` | `disk`, `format` (`exfat`, `fat32`, `ntfs`, `ext4`, `ext4-encrypted`), `name`, `scheme` (`gpt`, `mbr`) | `critical` |  | app (`Job::EraseDisk`, one job) | A |
| `disks.disconnect_volume` | `volume` | `disrupt`; a system disk `critical` |  | app; also locks an encrypted volume | B |
| `disks.erase_volume` | `volume`, `format`, `name` | `critical` |  | app | B |
| `disks.create_volume` | `disk`, `size`, `format`, `name` | `critical` | removes it while empty | app | B |
| `disks.delete_volume` | `volume` | `critical` |  | app | B |
| `disks.update_volume` | `volume`, `name`, `size` | `critical` | the earlier name | app | B |
| `disks.repair_volume` | `volume` | `critical` |  | app | B |
| `disks.connect_image` | `file` | read-only `disrupt`, else `critical` | `disks.disconnect_volume` | app | B |
| `disks.create_image` | `volume`, `name`, `within` | `critical` (reads everything) | `files.trash` | app | B |
| `disks.restore_image` | `image`, `disk` | `critical` |  | app; a bootable USB stick | B |

Phrases in Search: "eject {disk}".

A volume is known by its UUID together with its drive, a disk by serial and size, not by object
path, and serial numbers are not returned. The system disk, a disk in use by the system (mounted
outside the removable folders, swap, or a member of LVM, RAID or an open encrypted volume) and the
disk with the backup are refused whatever is allowed; the check is in the app's model and view today
and moves into the job. Unlocking asks its owner for the passphrase, and forced ejecting has no
intent.

"Reformat my USB stick so it works in Windows and Debian" is `disks.erase_disk` with `format`
`exfat`, `scheme` `gpt` and a name: one plan with the unmounting, the new table and the new volume,
and one Allow that names the stick, its size and what is on it. The system asks for no password for
a removable disk, and asks an administrator's for an internal one. `ext4` made this way needs its
owner set. After the new table is written it cannot be undone, and the prompt says so.
