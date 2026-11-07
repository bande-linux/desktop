# Install, first run and recovery

## What the user experiences
- **Mac:** one command in macOS Terminal, one size question, the recoveryOS step Apple requires, then the Mac starts Bande. No account or Wi-Fi questions there and no secrets on the EFI partition.
- **Other arm64 and amd64 machines:** write one image to a USB stick, boot it, pick "Install", pick a disk. It also runs from an attached disk in a VM.
- **First start, everywhere:** a full-screen assistant before any login: language, keyboard, Wi-Fi, account and password (which also protects the disk), appearance, privacy. Finishing goes straight into the desktop with no second login.
- **When the system is broken:** Recovery from the boot menu, or the same USB stick: Reinstall (keeps home), Restore from Backup, Disk Tools, Terminal.

## Design
One read-only image does all three jobs. `images/build.sh` builds a root tree with mmdebstrap from Debian testing and the Bande archive, and packs it as a squashfs.

| Job | How it boots the squashfs |
|---|---|
| USB installer | GPT: ESP + raw squashfs partition `BANDE-RECOVERY` |
| Recovery | The same partition on the installed disk, with a systemd-boot entry |
| Reinstall source | `bande-install` copies it into a new root subvolume, offline |

- **Live boot:** `boot=bande-live`, a small initramfs-tools boot script in the ordinary initramfs: squashfs below, tmpfs overlay above. So one kernel and initramfs serve both modes.
- **Disk layout** (`fix-update-safety` owns subvolumes and snapshots, `fix-security` owns encryption): GPT with ESP, `BANDE-RECOVERY`, `BANDE-ROOT` (LUKS2 + btrfs).
- **Mac image:** macOS cannot run cryptsetup, so `root.img` is a prebuilt LUKS2 volume with a well-known passphrase (`bande-provisional`). First run sets the real passphrase, removes that key and reencrypts online in the background, like FileVault. The USB install makes a fresh volume key and skips the reencrypt.
- **`bande-install`** (`images/rootfs`): the one engine for install and reinstall, a root script run through pkexec, reporting progress as lines on stdout.
- **Assistant:** `bande-setup`, the login-window role in `bande-compositor --greeter`, chosen while `/var/lib/bande/setup-pending` exists. It runs unprivileged as `_greetd`. Accounts, locale, time and Wi-Fi go through accountsservice, localed, timedated and NetworkManager, with a polkit rule that holds only while setup is pending. Disk steps go through pkexec. It signs in over greetd's IPC when done.
- **Recovery UI:** the same `bande-setup` binary, on the live system.

## Why this over the alternatives
- **Debian Installer or Calamares:** text UI or Qt, and neither gives first-run or recovery. A second system to maintain.
- **debootstrap at install time:** needs network and gives a different result each run. Reinstall must work offline.
- **Assistant as a session for a temporary user (GNOME's way):** leaves a throwaway user and needs a second login.
- **Separate recovery image:** a second build to keep working, and larger overall since the reinstall payload is needed anyway.
- **Signed Mac app in place of the Terminal command:** needs an Apple Developer ID, an owner decision; see Stages.

## Stages
1. Image build: archive kernel and packages, GitHub-hosted runners, generic and Apple targets, UEFI boot test in QEMU.
2. Install engine and layout: `bande-install`, btrfs, LUKS, live boot, USB image, proven in QEMU.
3. Assistant: `bande-setup`, driven in QEMU through keyboard input.
4. Recovery: reinstall keeps home, restore from `fix-backup`, disk tools, terminal.
5. Mac installer: new partitions and `installer_data.json`, setup questions removed.

Installing on the real mini is an owner step.
