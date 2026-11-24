# Disks

Part of [Bande Intents](../../intents.md). Things `disks.disk`, `disks.volume`. Owner:
`crates/services/disks-intents`. Work package D9.

| Intent | Parameters | Effects | Undo | Code | P |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `disks.find_disk`, `disks.find_volume` | `text` | `read` |  | app | B |
| `disks.get` | `things`, `properties` (size, free, health, temperature, mounted, formats offered, can be resized, protected) | `read` |  | part: health is in `bande-volumes`, the rest in the app's model | A |
| `disks.disconnect_volume` | `volume` | `disrupt` |  | app; also locks an encrypted volume | C |
| `disks.disconnect_disk` | `disk` (eject) | `disrupt` |  | ready | C |
| `disks.connect_volume` | `volume`; an encrypted one is asked by its owner | `disrupt` | `disks.disconnect_volume` | app (`jobs.rs`) | A |
| `disks.erase_disk` | `disk`, `format` (`exfat`, `fat32`, `ntfs`, `ext4`, `ext4-encrypted`), `name`, `scheme` (`gpt`, `mbr`) | `authorise` |  | app (`Job::EraseDisk`, one job) | A |
| `disks.erase_volume` | `volume`, `format`, `name` | `authorise` |  | app | A |
| `disks.create_volume` | `disk`, `size`, `format`, `name` | `authorise` | removes it while empty | app | B |
| `disks.delete_volume` | `volume` | `authorise` |  | app | B |
| `disks.update_volume` | `volume`, `name`, `size` | name `disrupt`, size `authorise` | the earlier name | app | B |
| `disks.repair_volume` | `volume` | `authorise` |  | app | B |
| `disks.connect_image` | `file` | read-only `disrupt`, else `authorise` | `disks.disconnect_volume` | app | B |
| `disks.create_image` | `volume`, `name`, `within` | `authorise` (reads everything) | `files.trash` | app | B |
| `disks.restore_image` | `image`, `disk` | `authorise` |  | app; a bootable USB stick | A |

A volume is known by its UUID together with its drive, a disk by serial and size, not by object
path, and serial numbers are not returned. The system disk, a disk in use by the system (mounted
outside the removable folders, swap, or a member of LVM, RAID or an open encrypted volume) and the
disk with the backup are refused whatever is authorised; the check is in the app's model and view
today and moves into the job. Unlocking asks its owner for the passphrase, and forced ejecting has
no intent.

"Reformat my USB stick so it works in Windows and Debian" is `disks.erase_disk` with `format`
`exfat`, `scheme` `gpt` and a name: one plan with the unmounting, the new table and the new volume,
one prompt that names the stick, its size and what is on it, and one password. It asks for none
today, since the backend does not for a removable disk. `ext4` made this way needs its owner set.
After the new table is written it cannot be undone, and the prompt says so.
