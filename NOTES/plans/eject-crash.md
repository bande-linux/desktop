# Eject crashed a Debian VM (bande-desktop 0.1.1)

Ejecting a "USB drive" holding the Debian ISO took the VM down. Delete this file when fixed.

## What Bande does today (0.1.1 = main)

`crates/libs/volumes/src/eject.rs:31-79`, for one mount point: `Filesystem.Unmount` (Force
Eject adds `force`, a lazy unmount), then **always** `Drive.PowerOff`, ignoring `Ejectable`,
`CanPowerOff`, `Optical` and the drive's other volumes; a PowerOff failure is only logged (`:68`).
udisks 2.11 `handle_power_off` does not check `CanPowerOff`: it fsyncs and opens every block
`O_EXCL`, sends SCSI SYNCHRONIZE CACHE and START STOP UNIT (stop), then writes `1` to the
parent USB device's sysfs `remove`, i.e. the guest kernel disables that USB port.
GNOME/KDE "Eject" call `Drive.Eject` (SCSI eject, device stays) when `Ejectable`; PowerOff is
their separate "Safely Remove / Power Off". UTM attaches the installer ISO as a removable
USB mass-storage device (QEMU `usb-storage,removable=true`; Apple VZ `VZUSBMassStorageDevice`),
so `Ejectable=true` and Bande takes the rare guest-initiated USB port-removal path.

## Likely causes, ranked

1. **Hypervisor dies on guest USB port removal** (most likely; "the whole VM"). Bande's
   PowerOff writes sysfs `remove` on the emulated xHCI port, a path GNOME users rarely hit.
2. **Guest kernel oops/panic** in usb-storage/xhci teardown with a busy or lazily unmounted
   iso9660 (Force Eject). Serial console tells.
3. **Bande process crash** (session only): I found no panic site. The eject path, automount,
   Files `refresh_volumes` (`files/src/view.rs:446`), the desktop lister and `bande-watch`
   (handles `IGNORED`) have no unwrap/expect/indexing, and nothing mmaps files from the volume.
   There is no remount loop: automount keeps the volume entry, so it never remounts after an eject.
4. CD-ROM (sr0, non-USB): PowerOff only stops the unit, then fails with "No usb device". The
   disc stays inserted but unmounted, and Bande can't remount it. That's a bug but not a crash.

Not a crash, but wrong: a hybrid ISO on a stick shows two volumes (iso9660 + ESP). Ejecting
one leaves the other mounted, so PowerOff fails ("drive in use") silently.

## Ask the owner to collect (on the VM, after the next crash or now from `-b -1`)

- Hypervisor and backend (UTM QEMU or Apple Virtualization), how the ISO is attached
  (USB/removable, or CD/DVD), and what was seen: VM window closed/"stopped", black screen,
  back to greeter, or frozen. Host side: UTM debug log / QEMU stderr, or macOS
  `~/Library/Logs/DiagnosticReports/*Virtualization*`.
- `journalctl -b -1 -o short-precise | tail -300`, `journalctl -b -1 -u udisks2` (look for
  "Powered off ... wrote to sysfs path .../remove" as the last lines), `journalctl -b -1 -k`.
- `coredumpctl list` (bande-shell, -compositor, -files, -automount).
- Guest serial console output, if any (kernel panic).
- With the ISO attached: `lsblk -o NAME,TRAN,RM,TYPE,FSTYPE,LABEL,MOUNTPOINTS` and
  `udisksctl info -b /dev/<dev>` (Ejectable, CanPowerOff, ConnectionBus, Optical).
- Exact UI path: Files sidebar eject button, Ctrl+E, desktop icon > Eject, or Force Eject.

## Fix (correct regardless of which cause it was)

In `bande-volumes::eject`, resolve the volume's drive and then:
1. Unmount **every** mounted filesystem whose `Block.Drive` is that drive (with `force` if
   asked). For loop-backed volumes (`Drive == "/"`), unmount, then `Loop.Delete` only if
   `Loop.SetupByUID` is ours.
2. `Drive.Ejectable` (optical, removable USB/ISO media): `Drive.Eject`. This is a tray eject,
   like macOS on optical media and GNOME's Eject.
   Not ejectable but `CanPowerOff` (USB HDD/SSD): `PowerOff`, like macOS ejecting a USB disk.
   Neither: unmount only.
3. Report an Eject/PowerOff failure through the existing `failed` callback, not just log it.
Size: about 70 lines in `eject.rs`. Add Block.Drive, a Drive (Ejectable/CanPowerOff,
records Eject/PowerOff) and Unmount to `test-bus/src/drives.rs` (~50 lines). One test in
`crates/libs/volumes/tests` (~40 lines), with `DBUS_SYSTEM_BUS_ADDRESS` set to the test bus: an
ejectable drive with two volumes unmounts both and calls Eject, never PowerOff.
Verify: owner ejects the UTM ISO from Files and from the desktop. The VM survives, the volume
disappears, and udisks logs `eject`, not `remove`. A USB stick on the mini still disappears.

## Owner decisions

- Eject vs power off: **recommend Eject when `Ejectable`**, PowerOff only for non-ejectable
  USB disks, matching macOS on optical media and GNOME's Eject. The cost: an ejectable USB stick
  stays enumerated (media removed) instead of losing port power.
- Multi-volume disks: macOS asks "Eject All / Eject". **Recommend ejecting all volumes now**
  (always safe), and add the Finder prompt later if wanted.
